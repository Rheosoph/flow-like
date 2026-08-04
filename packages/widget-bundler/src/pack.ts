import { createHash } from "node:crypto";
import {
	existsSync,
	mkdirSync,
	readFileSync,
	readdirSync,
	statSync,
	writeFileSync,
} from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { type ZipOptions, type Zippable, gzipSync, zipSync } from "fflate";
import { parse as parseToml } from "smol-toml";
import {
	BUNDLE_FORMAT_VERSION,
	BUNDLE_MANIFEST_PATH,
	type BundleSharedEntry,
	type BundleWidgetEntry,
	type WidgetBundleManifest,
	manifestToJson,
} from "./bundle-format";
import { WIDGET_PROTOCOL, contractToJson } from "./contract-types";
import { buildCsp, injectCspMeta } from "./csp";
import { extractContract } from "./extract";
import { injectContractScript, inlineHtml } from "./inline";

export interface PackOptions {
	out?: string;
	servingPrefix?: string | null;
	connectHosts?: string[];
	createdAt?: string;
	quiet?: boolean;
}

export interface PackResult {
	outPath: string;
	bytes: Uint8Array;
	/** Whole-file sha256 hex (the manifest `widget_bundle_hash`) */
	hash: string;
	manifest: WidgetBundleManifest;
	warnings: string[];
	report: string;
}

export interface PackageInfo {
	id: string;
	version: string;
}

/** Fixed ZIP entry mtime (1980-01-01) for deterministic archives */
export const ZIP_ENTRY_MTIME_MS = 315532800000;

export function sha256Hex(data: Uint8Array): string {
	return createHash("sha256").update(data).digest("hex");
}

export function entryHash(data: Uint8Array): string {
	return `sha256:${sha256Hex(data)}`;
}

/**
 * Read the package `id`/`version` from `flow-like.toml`; accepts the manifest
 * form (top-level keys) and the template form (`[package]` table).
 */
export function readPackageInfo(projectDir: string): PackageInfo {
	const tomlPath = join(projectDir, "flow-like.toml");
	if (!existsSync(tomlPath)) {
		throw new Error(`No flow-like.toml found in ${resolve(projectDir)}`);
	}
	const doc = parseToml(readFileSync(tomlPath, "utf8")) as Record<
		string,
		unknown
	>;
	const pkg =
		typeof doc.package === "object" && doc.package !== null
			? (doc.package as Record<string, unknown>)
			: {};
	const id = typeof doc.id === "string" ? doc.id : pkg.id;
	const version = typeof doc.version === "string" ? doc.version : pkg.version;
	if (typeof id !== "string" || id.length === 0) {
		throw new Error(
			`${tomlPath} is missing the package 'id' (top-level or [package].id)`,
		);
	}
	if (typeof version !== "string" || version.length === 0) {
		throw new Error(
			`${tomlPath} is missing the package 'version' (top-level or [package].version)`,
		);
	}
	return { id, version };
}

export interface FrameworkGroup {
	name: string;
	dir: string;
	framework: string;
}

const FRAMEWORK_DEPENDENCIES: [string, string][] = [
	["svelte", "svelte"],
	["vue", "vue"],
	["solid-js", "solid"],
	["preact", "preact"],
	["lit", "lit"],
	["react", "react"],
];

export function discoverGroups(projectDir: string): FrameworkGroup[] {
	const widgetsDir = join(projectDir, "widgets");
	if (!existsSync(widgetsDir)) return [];
	const groups: FrameworkGroup[] = [];
	for (const name of readdirSync(widgetsDir).sort()) {
		const dir = join(widgetsDir, name);
		if (!statSync(dir).isDirectory()) continue;
		const packageJsonPath = join(dir, "package.json");
		if (!existsSync(packageJsonPath)) continue;
		groups.push({
			name,
			dir,
			framework: detectFramework(packageJsonPath, name),
		});
	}
	return groups;
}

function detectFramework(packageJsonPath: string, fallback: string): string {
	try {
		const pkg = JSON.parse(readFileSync(packageJsonPath, "utf8")) as {
			dependencies?: Record<string, string>;
			devDependencies?: Record<string, string>;
		};
		const deps = { ...pkg.dependencies, ...pkg.devDependencies };
		for (const [dep, framework] of FRAMEWORK_DEPENDENCIES) {
			if (deps[dep]) return framework;
		}
	} catch {
		// informational only; fall through to the directory name
	}
	return fallback;
}

export function discoverGroupWidgets(groupDir: string): string[] {
	const widgetsDir = join(groupDir, "src", "widgets");
	if (!existsSync(widgetsDir)) return [];
	return readdirSync(widgetsDir)
		.sort()
		.filter(
			(name) =>
				statSync(join(widgetsDir, name)).isDirectory() &&
				existsSync(join(widgetsDir, name, "widget.config.ts")),
		);
}

function findBuiltDocument(
	groupDir: string,
	widgetId: string,
	groupWidgetCount: number,
): string {
	const candidates = [
		join(groupDir, "dist", "widgets", widgetId, "index.html"),
		join(groupDir, "dist", "src", "widgets", widgetId, "index.html"),
	];
	if (groupWidgetCount === 1) {
		candidates.push(join(groupDir, "dist", "index.html"));
	}
	for (const candidate of candidates) {
		if (existsSync(candidate)) return candidate;
	}
	throw new Error(
		`No built document found for widget '${widgetId}'. Tried:\n${candidates
			.map((c) => `  - ${c}`)
			.join(
				"\n",
			)}\nRun \`bun run build\` in ${groupDir} first (the group's build script must emit dist/).`,
	);
}

function collectSharedFiles(distDir: string): Map<string, Uint8Array> {
	const sharedDir = join(distDir, "shared");
	const files = new Map<string, Uint8Array>();
	if (!existsSync(sharedDir)) return files;
	const walk = (dir: string) => {
		for (const name of readdirSync(dir).sort()) {
			const full = join(dir, name);
			if (statSync(full).isDirectory()) {
				walk(full);
			} else {
				const rel = relative(sharedDir, full).split(sep).join("/");
				files.set(`shared/${rel}`, new Uint8Array(readFileSync(full)));
			}
		}
	};
	walk(sharedDir);
	return files;
}

function formatSize(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
	return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

function resolveCreatedAt(explicit: string | undefined): string | undefined {
	if (explicit !== undefined) return explicit;
	const epoch = process.env.SOURCE_DATE_EPOCH;
	if (!epoch) return undefined;
	const seconds = Number(epoch);
	if (!Number.isFinite(seconds)) return undefined;
	return new Date(seconds * 1000).toISOString();
}

interface PackedWidget {
	id: string;
	name: string;
	description: string;
	framework: string;
	html: Uint8Array;
	contractJson: Uint8Array;
	assets: string[];
	gzipSize: number;
}

/** Build the `.flwb` widget bundle for a scaffolded package project. */
export async function pack(
	projectDir: string,
	opts: PackOptions = {},
): Promise<PackResult> {
	const project = resolve(projectDir);
	const info = readPackageInfo(project);
	const groups = discoverGroups(project);
	if (groups.length === 0) {
		throw new Error(
			`No framework groups found under ${join(project, "widgets")} (expected widgets/<group>/package.json)`,
		);
	}

	const csp = buildCsp(opts.servingPrefix ?? null, opts.connectHosts ?? []);
	const warnings: string[] = [];
	const widgets: PackedWidget[] = [];
	const shared = new Map<string, Uint8Array>();
	const seenIds = new Set<string>();

	for (const group of groups) {
		const widgetIds = discoverGroupWidgets(group.dir);
		if (widgetIds.length === 0) continue;
		const distDir = resolve(group.dir, "dist");
		const groupShared = collectSharedFiles(distDir);

		for (const widgetId of widgetIds) {
			const configPath = join(
				group.dir,
				"src",
				"widgets",
				widgetId,
				"widget.config.ts",
			);
			const extracted = extractContract(configPath);
			warnings.push(...extracted.warnings);
			if (extracted.config.id !== widgetId) {
				throw new Error(
					`Widget id '${extracted.config.id}' in ${configPath} does not match its directory name '${widgetId}'`,
				);
			}
			if (seenIds.has(widgetId)) {
				throw new Error(
					`Duplicate widget id across framework groups: ${widgetId}`,
				);
			}
			seenIds.add(widgetId);

			const htmlPath = findBuiltDocument(group.dir, widgetId, widgetIds.length);
			const htmlDir = dirname(htmlPath);
			const resolveAsset = (relPath: string): Uint8Array | null => {
				const full = resolve(htmlDir, relPath);
				if (full !== distDir && !full.startsWith(distDir + sep)) return null;
				if (!existsSync(full) || !statSync(full).isFile()) return null;
				return new Uint8Array(readFileSync(full));
			};

			let inlineResult: ReturnType<typeof inlineHtml>;
			try {
				inlineResult = inlineHtml(readFileSync(htmlPath, "utf8"), resolveAsset);
			} catch (e) {
				throw new Error(
					`Failed to inline ${htmlPath}: ${e instanceof Error ? e.message : e}`,
				);
			}

			for (const asset of inlineResult.external) {
				const chunk = groupShared.get(asset);
				if (!chunk) {
					throw new Error(
						`Widget '${widgetId}' references '${asset}' but ${join(distDir, asset)} does not exist`,
					);
				}
				const existing = shared.get(asset);
				if (existing && entryHash(existing) !== entryHash(chunk)) {
					throw new Error(
						`Shared chunk collision: '${asset}' has different content across framework groups`,
					);
				}
			}
			for (const [path, data] of groupShared) {
				const existing = shared.get(path);
				if (existing && entryHash(existing) !== entryHash(data)) {
					throw new Error(
						`Shared chunk collision: '${path}' has different content across framework groups`,
					);
				}
				shared.set(path, data);
			}

			let html = injectContractScript(inlineResult.html, extracted.contract);
			html = injectCspMeta(html, csp);
			const htmlBytes = new TextEncoder().encode(html);

			widgets.push({
				id: widgetId,
				name: extracted.config.name,
				description: extracted.config.description,
				framework: group.framework,
				html: htmlBytes,
				contractJson: new TextEncoder().encode(
					contractToJson(extracted.contract),
				),
				assets: inlineResult.external,
				gzipSize: gzipSync(htmlBytes, { level: 6, mtime: 0 }).length,
			});
		}
	}

	if (widgets.length === 0) {
		throw new Error(
			`No widgets found (expected widgets/<group>/src/widgets/<id>/widget.config.ts under ${project})`,
		);
	}
	widgets.sort((a, b) => (a.id < b.id ? -1 : a.id > b.id ? 1 : 0));

	const sharedEntries: BundleSharedEntry[] = [...shared.keys()]
		.sort()
		.map((path) => ({ path, hash: entryHash(shared.get(path) as Uint8Array) }));

	const widgetEntries: BundleWidgetEntry[] = widgets.map((widget) => ({
		id: widget.id,
		name: widget.name,
		description: widget.description,
		entry: `widgets/${widget.id}/index.html`,
		contract: `widgets/${widget.id}/contract.json`,
		entryHash: entryHash(widget.html),
		assets: widget.assets,
		framework: widget.framework,
		sizeHint: { raw: widget.html.length, gzip: widget.gzipSize },
	}));

	const createdAt = resolveCreatedAt(opts.createdAt);
	const manifest: WidgetBundleManifest = {
		formatVersion: BUNDLE_FORMAT_VERSION,
		packageId: info.id,
		packageVersion: info.version,
		protocol: WIDGET_PROTOCOL,
		...(createdAt !== undefined && { createdAt }),
		shared: sharedEntries,
		widgets: widgetEntries,
	};

	const entries = new Map<string, Uint8Array>();
	entries.set(
		BUNDLE_MANIFEST_PATH,
		new TextEncoder().encode(manifestToJson(manifest)),
	);
	for (const [path, data] of shared) entries.set(path, data);
	for (const widget of widgets) {
		entries.set(`widgets/${widget.id}/index.html`, widget.html);
		entries.set(`widgets/${widget.id}/contract.json`, widget.contractJson);
	}

	const zipOptions: ZipOptions = {
		level: 6,
		mtime: new Date(ZIP_ENTRY_MTIME_MS),
	};
	const zippable: Zippable = {};
	for (const path of [...entries.keys()].sort()) {
		zippable[path] = [entries.get(path) as Uint8Array, zipOptions];
	}
	const bytes = zipSync(zippable);
	const hash = sha256Hex(bytes);

	const outPath = resolve(opts.out ?? join(project, "widgets.flwb"));
	mkdirSync(dirname(outPath), { recursive: true });
	writeFileSync(outPath, bytes);

	const lines: string[] = [];
	for (const widget of widgets) {
		lines.push(
			`widget ${widget.id.padEnd(24)} raw ${formatSize(widget.html.length).padStart(9)}  gzip ${formatSize(widget.gzipSize).padStart(9)}`,
		);
	}
	for (const [path, data] of [...shared.entries()].sort(([a], [b]) =>
		a < b ? -1 : 1,
	)) {
		lines.push(
			`shared ${path.padEnd(24)} raw ${formatSize(data.length).padStart(9)}`,
		);
	}
	lines.push(`bundle ${outPath}`);
	lines.push(
		`total ${formatSize(bytes.length)} (${widgets.length} widget${widgets.length === 1 ? "" : "s"}, ${shared.size} shared chunk${shared.size === 1 ? "" : "s"})`,
	);
	lines.push(`sha256 ${hash}`);
	const report = lines.join("\n");
	if (!opts.quiet) console.log(report);

	return { outPath, bytes, hash, manifest, warnings, report };
}
