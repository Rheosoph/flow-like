#!/usr/bin/env bun

import { readFile, readdir, unlink } from "node:fs/promises";
import { basename, dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const SCRIPT_DIR = dirname(fileURLToPath(import.meta.url));
const REPO_ROOT = resolve(SCRIPT_DIR, "..");
const TAURI_CONF = resolve(REPO_ROOT, "apps/desktop/src-tauri/tauri.conf.json");

/** Mirrors `MAX_SOURCE_MAP_BYTES` in packages/api/src/routes/admin/telemetry/sourcemaps.rs. */
const MAX_MAP_BYTES = 20 * 1024 * 1024;
/** Mirrors `SOURCE_MAP_BODY_LIMIT_BYTES`; JSON escaping inflates the map well past its file size. */
const MAX_BODY_BYTES = 24 * 1024 * 1024;
const REQUEST_TIMEOUT_MS = 60_000;
const RETRY_DELAY_MS = 1_000;
const UPLOAD_PATH = "api/v1/admin/telemetry/sourcemaps";
const SOURCES = ["web", "desktop"] as const;

type UploadSource = (typeof SOURCES)[number];

interface CliOptions {
	source?: UploadSource;
	dir?: string;
	deleteMaps: boolean;
	help: boolean;
}

interface SourceMapJson {
	file?: unknown;
	mappings?: unknown;
	sections?: unknown;
}

interface PreparedUpload {
	path: string;
	fileName: string;
	body: string;
}

interface UploadFailure {
	fileName: string;
	reason: string;
}

class CliError extends Error {}

export function usage(): string {
	return `Upload build source maps to the internal telemetry backend

Usage:
  bun scripts/upload-sourcemaps.ts --source=web --dir=out
  bun scripts/upload-sourcemaps.ts --source=desktop --dir=out --delete-maps

Options:
  --source <web|desktop>  Telemetry source the build belongs to (required)
  --dir <path>            Build output directory to scan for .map files (required)
  --delete-maps           Delete each .map file after it uploaded successfully
  --help                  Show this help

Environment:
  FLOW_LIKE_API_URL        API origin, no /api/v1 suffix (e.g. https://api.flow-like.com)
  FLOW_LIKE_SOURCEMAP_TOKEN  PAT ("pat_<id>.<secret>") or access token of an Admin user
  FLOW_LIKE_RELEASE        Release the maps belong to — see the contract below

  With FLOW_LIKE_API_URL or FLOW_LIKE_SOURCEMAP_TOKEN unset the script prints one
  line and exits 0, so a normal build without upload credentials is unaffected.

Release contract (the single most likely way symbolication dies silently):
  The server resolves maps for a crash with
    release = <error event's release>  AND  source = <error event's source>,
  then matches the file by BASENAME only. A release string that differs by even
  one character never matches and every stack trace stays minified — with no
  error anywhere. FLOW_LIKE_RELEASE must therefore be byte-identical to the
  release the client reports at runtime:

    --source=desktop  release = getVersion() from "@tauri-apps/api/app", which
                      is the "version" field of
                      apps/desktop/src-tauri/tauri.conf.json (currently 0.1.7).
                      Both apps/desktop/components/telemetry-provider.tsx and
                      the buffered native crash drain report exactly that value.
    --source=web      release = the "release" passed to createTelemetryClient()
                      in apps/web/components/telemetry-provider.tsx. That
                      provider passes no appVersion and no release today, so web
                      crash events carry release=null and NOTHING uploaded here
                      can match until the provider reports the same string that
                      is exported as FLOW_LIKE_RELEASE for the build.

  For --source=desktop this script compares FLOW_LIKE_RELEASE against
  tauri.conf.json and warns on a mismatch.

Exit codes:
  0  everything uploaded, or skipped because the URL or token is unset
  1  at least one upload failed after its retry
  2  usage or configuration error
`;
}

function parseArgs(argv: readonly string[]): CliOptions {
	const options: CliOptions = { deleteMaps: false, help: false };
	let index = 0;
	while (index < argv.length) {
		const arg = argv[index++];
		const separator = arg.indexOf("=");
		const flag = separator === -1 ? arg : arg.slice(0, separator);
		const readValue = (): string => {
			const value = separator === -1 ? argv[index++] : arg.slice(separator + 1);
			if (!value) throw new CliError(`${flag} requires a value`);
			return value;
		};

		switch (flag) {
			case "--help":
			case "-h":
				options.help = true;
				break;
			case "--delete-maps":
				options.deleteMaps = true;
				break;
			case "--source": {
				const value = readValue();
				if (!SOURCES.includes(value as UploadSource)) {
					throw new CliError(
						`--source must be one of ${SOURCES.join(", ")}, got "${value}"`,
					);
				}
				options.source = value as UploadSource;
				break;
			}
			case "--dir":
				options.dir = readValue();
				break;
			default:
				throw new CliError(`Unknown argument "${arg}"`);
		}
	}
	return options;
}

/** Accepts an origin with or without a trailing `/api/v1`, matching getApiOrigin(). */
function uploadEndpoint(apiUrl: string): string {
	const origin = apiUrl
		.trim()
		.replace(/\/+$/, "")
		.replace(/\/api\/v1$/, "");
	return `${origin}/${UPLOAD_PATH}`;
}

async function walkSourceMaps(dir: string): Promise<string[]> {
	const found: string[] = [];
	const visit = async (current: string) => {
		const entries = await readdir(current, { withFileTypes: true });
		for (const entry of entries) {
			if (entry.isSymbolicLink()) continue;
			const full = join(current, entry.name);
			if (entry.isDirectory()) await visit(full);
			else if (entry.isFile() && entry.name.endsWith(".map")) found.push(full);
		}
	};
	await visit(dir);
	return found.sort();
}

/**
 * The read path matches a stored map to a frame by basename, so the minified
 * file name is what must be uploaded — never the path inside the build output.
 */
function minifiedFileName(mapPath: string, parsed: SourceMapJson): string {
	const stripped = basename(mapPath).replace(/\.map$/, "");
	if (/\.(?:js|mjs|cjs)$/.test(stripped)) return stripped;
	const declared = typeof parsed.file === "string" ? basename(parsed.file) : "";
	return declared || stripped;
}

/** The server rejects maps without usable mappings; catching it here saves a round trip. */
function hasMappings(parsed: SourceMapJson): boolean {
	if (Array.isArray(parsed.sections)) return parsed.sections.length > 0;
	return typeof parsed.mappings === "string" && parsed.mappings.length > 0;
}

async function prepareUpload(
	path: string,
	release: string,
	source: UploadSource,
): Promise<PreparedUpload | string> {
	const map = await readFile(path, "utf-8");
	const bytes = Buffer.byteLength(map);
	if (bytes > MAX_MAP_BYTES) {
		return `larger than the ${MAX_MAP_BYTES} byte server limit (${bytes} bytes)`;
	}

	let parsed: SourceMapJson;
	try {
		parsed = JSON.parse(map) as SourceMapJson;
	} catch (error) {
		return `not readable JSON (${(error as Error).message})`;
	}
	if (!hasMappings(parsed)) return "contains no mappings";

	const fileName = minifiedFileName(path, parsed);
	const body = JSON.stringify({ release, source, file_name: fileName, map });
	if (Buffer.byteLength(body) > MAX_BODY_BYTES) {
		return `exceeds the ${MAX_BODY_BYTES} byte request limit once encoded`;
	}
	return { path, fileName, body };
}

async function postSourceMap(
	endpoint: string,
	token: string,
	body: string,
): Promise<{ ok: boolean; permanent: boolean; reason: string }> {
	try {
		const response = await fetch(endpoint, {
			method: "POST",
			headers: {
				"content-type": "application/json",
				authorization: `Bearer ${token}`,
			},
			body,
			signal: AbortSignal.timeout(REQUEST_TIMEOUT_MS),
		});
		if (response.ok) return { ok: true, permanent: false, reason: "" };
		const detail = (await response.text().catch(() => "")).slice(0, 200).trim();
		return {
			ok: false,
			// A rejected payload or token stays rejected; only 408/429 are worth retrying.
			permanent:
				response.status >= 400 &&
				response.status < 500 &&
				response.status !== 408 &&
				response.status !== 429,
			reason: `HTTP ${response.status}${detail ? ` — ${detail}` : ""}`,
		};
	} catch (error) {
		return { ok: false, permanent: false, reason: (error as Error).message };
	}
}

async function uploadWithRetry(
	endpoint: string,
	token: string,
	upload: PreparedUpload,
): Promise<string | undefined> {
	const first = await postSourceMap(endpoint, token, upload.body);
	if (first.ok) return undefined;
	if (first.permanent) return first.reason;

	await new Promise((done) => setTimeout(done, RETRY_DELAY_MS));
	const retry = await postSourceMap(endpoint, token, upload.body);
	return retry.ok ? undefined : `${retry.reason} (retried once)`;
}

async function warnOnDesktopReleaseMismatch(release: string): Promise<void> {
	try {
		const conf = JSON.parse(await readFile(TAURI_CONF, "utf-8")) as {
			version?: unknown;
		};
		if (typeof conf.version !== "string" || conf.version === release) return;
		console.warn(
			`[sourcemaps] WARNING: FLOW_LIKE_RELEASE="${release}" but tauri.conf.json reports "${conf.version}".`,
		);
		console.warn(
			"[sourcemaps] The desktop client reports the tauri.conf.json version; uploaded maps will never match.",
		);
	} catch {
		// Running outside the repo checkout is fine; the warning is a courtesy.
	}
}

async function run(argv: readonly string[]): Promise<number> {
	let options: CliOptions;
	try {
		options = parseArgs(argv);
	} catch (error) {
		console.error(`[sourcemaps] ${(error as Error).message}\n`);
		console.error(usage());
		return 2;
	}

	if (options.help) {
		console.log(usage());
		return 0;
	}

	const apiUrl = process.env.FLOW_LIKE_API_URL?.trim();
	const token = process.env.FLOW_LIKE_SOURCEMAP_TOKEN?.trim();
	if (!apiUrl || !token) {
		console.log(
			"[sourcemaps] FLOW_LIKE_API_URL or FLOW_LIKE_SOURCEMAP_TOKEN not set — skipping upload.",
		);
		return 0;
	}

	if (!options.source) {
		console.error("[sourcemaps] --source is required\n");
		console.error(usage());
		return 2;
	}
	if (!options.dir) {
		console.error("[sourcemaps] --dir is required\n");
		console.error(usage());
		return 2;
	}
	const release = process.env.FLOW_LIKE_RELEASE?.trim();
	if (!release) {
		console.error(
			"[sourcemaps] FLOW_LIKE_RELEASE is required and must equal the release the client reports.",
		);
		console.error("[sourcemaps] Run with --help for the release contract.");
		return 2;
	}
	if (options.source === "desktop") await warnOnDesktopReleaseMismatch(release);

	const dir = resolve(process.cwd(), options.dir);
	let mapPaths: string[];
	try {
		mapPaths = await walkSourceMaps(dir);
	} catch (error) {
		console.error(
			`[sourcemaps] Cannot read --dir "${dir}": ${(error as Error).message}`,
		);
		return 2;
	}
	if (mapPaths.length === 0) {
		console.log(
			`[sourcemaps] No .map files under ${dir} — nothing to upload. Source map generation is off by default in Next.js production builds.`,
		);
		return 0;
	}

	const endpoint = uploadEndpoint(apiUrl);
	console.log(
		`[sourcemaps] Uploading ${mapPaths.length} map(s) from ${dir} as release "${release}" (source ${options.source}) to ${endpoint}`,
	);

	const started = Date.now();
	const seen = new Map<string, string>();
	const failures: UploadFailure[] = [];
	const uploaded: PreparedUpload[] = [];
	let skipped = 0;

	for (const path of mapPaths) {
		const shown = relative(dir, path);
		const prepared = await prepareUpload(path, release, options.source);
		if (typeof prepared === "string") {
			skipped++;
			console.warn(`[sourcemaps] skip ${shown}: ${prepared}`);
			continue;
		}
		const duplicate = seen.get(prepared.fileName);
		if (duplicate) {
			skipped++;
			console.warn(
				`[sourcemaps] skip ${shown}: basename "${prepared.fileName}" is already taken by ${duplicate}`,
			);
			continue;
		}
		seen.set(prepared.fileName, shown);

		const failure = await uploadWithRetry(endpoint, token, prepared);
		if (failure) {
			failures.push({ fileName: prepared.fileName, reason: failure });
			console.error(`[sourcemaps] fail ${shown}: ${failure}`);
			continue;
		}
		uploaded.push(prepared);
	}

	let deleted = 0;
	if (options.deleteMaps) {
		for (const upload of uploaded) {
			try {
				await unlink(upload.path);
				deleted++;
			} catch (error) {
				console.warn(
					`[sourcemaps] could not delete ${relative(dir, upload.path)}: ${(error as Error).message}`,
				);
			}
		}
	}

	const elapsed = ((Date.now() - started) / 1000).toFixed(1);
	console.log(
		`[sourcemaps] Done in ${elapsed}s — uploaded ${uploaded.length}, skipped ${skipped}, failed ${failures.length}${
			options.deleteMaps ? `, deleted ${deleted}` : ""
		}.`,
	);
	if (options.deleteMaps && deleted < mapPaths.length) {
		console.warn(
			`[sourcemaps] ${mapPaths.length - deleted} map(s) remain in the build output and would be served publicly.`,
		);
	}
	for (const failure of failures) {
		console.error(`[sourcemaps]   ${failure.fileName}: ${failure.reason}`);
	}
	return failures.length > 0 ? 1 : 0;
}

process.exit(await run(process.argv.slice(2)));
