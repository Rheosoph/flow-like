import { type Dirent, existsSync } from "node:fs";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import path from "node:path";
import type { Connect, Plugin, PreviewServer, ViteDevServer } from "vite";

/**
 * The studio has no backend. Its only privileged operation is reading and
 * writing the JSON files in `packages/locales/locales`, which the Vite dev
 * server it already runs inside can do directly — so this middleware is the
 * whole "server". It refuses to touch anything outside that directory.
 */

const LOCALES_DIR = path.resolve(
	import.meta.dirname,
	"../../../packages/locales/locales",
);
const CONFIG_PATH = path.join(LOCALES_DIR, "config.json");

/** Tabs and a trailing newline, so writes match what biome would produce. */
function serialize(value: unknown): string {
	return `${JSON.stringify(value, null, "\t")}\n`;
}

export interface LocaleConfig {
	sourceLanguage: string;
	defaultNamespace: string;
	namespaces: string[];
	languages: string[];
}

async function readConfig(): Promise<LocaleConfig> {
	return JSON.parse(await readFile(CONFIG_PATH, "utf8"));
}

async function readNamespace(
	language: string,
	namespace: string,
): Promise<Record<string, unknown>> {
	const file = path.join(LOCALES_DIR, language, `${namespace}.json`);
	if (!existsSync(file)) return {};
	const raw = await readFile(file, "utf8");
	return raw.trim() ? JSON.parse(raw) : {};
}

async function readAll() {
	const config = await readConfig();
	const bundles: Record<string, Record<string, Record<string, unknown>>> = {};

	for (const language of config.languages) {
		bundles[language] = {};
		for (const namespace of config.namespaces) {
			bundles[language][namespace] = await readNamespace(language, namespace);
		}
	}

	return { config, bundles };
}

/** Rejects `..`, absolute paths and anything that is not a known name. */
function assertKnown(value: string, allowed: string[], what: string): void {
	if (!allowed.includes(value)) {
		throw new Error(`Unknown ${what}: ${value}`);
	}
}

function isLanguageCode(value: string): boolean {
	return /^[a-z]{2,3}(-[A-Za-z0-9]{2,8})*$/.test(value);
}

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

function setOwn(
	target: Record<string, unknown>,
	key: string,
	value: unknown,
): void {
	Object.defineProperty(target, key, {
		value,
		enumerable: true,
		configurable: true,
		writable: true,
	});
}

/** Locale bundles may contain nested objects, but every leaf must be a string. */
function assertLocaleTree(value: unknown, pathLabel = "body"): void {
	if (!isRecord(value)) throw new Error(`${pathLabel} must be a JSON object`);
	for (const [key, child] of Object.entries(value)) {
		if (typeof child === "string") continue;
		if (isRecord(child)) {
			assertLocaleTree(child, `${pathLabel}.${key}`);
			continue;
		}
		throw new Error(`${pathLabel}.${key} must be a string or object`);
	}
}

/** Keep the exact source keyset, retain empty translations, and drop orphans. */
export function normalizeTargetTree(
	source: Record<string, unknown>,
	target: Record<string, unknown>,
): Record<string, unknown> {
	const normalized: Record<string, unknown> = {};
	for (const [key, sourceValue] of Object.entries(source)) {
		const targetValue = Object.hasOwn(target, key) ? target[key] : undefined;
		if (typeof sourceValue === "string") {
			setOwn(
				normalized,
				key,
				typeof targetValue === "string" ? targetValue : "",
			);
			continue;
		}
		if (isRecord(sourceValue)) {
			setOwn(
				normalized,
				key,
				normalizeTargetTree(
					sourceValue,
					isRecord(targetValue) ? targetValue : {},
				),
			);
		}
	}
	return normalized;
}

async function writeNamespace(
	language: string,
	namespace: string,
	tree: Record<string, unknown>,
): Promise<void> {
	const config = await readConfig();
	assertKnown(language, config.languages, "language");
	assertKnown(namespace, config.namespaces, "namespace");
	if (language === config.sourceLanguage) {
		throw new Error("The source language is read-only in Translation Studio");
	}
	assertLocaleTree(tree);
	const source = await readNamespace(config.sourceLanguage, namespace);

	const dir = path.join(LOCALES_DIR, language);
	await mkdir(dir, { recursive: true });
	await writeFile(
		path.join(dir, `${namespace}.json`),
		serialize(normalizeTargetTree(source, tree)),
		"utf8",
	);
}

async function addLanguage(language: string): Promise<LocaleConfig> {
	if (!isLanguageCode(language)) {
		throw new Error(
			`"${language}" is not a BCP-47 language code (expected e.g. "fr" or "pt-BR")`,
		);
	}

	const config = await readConfig();
	if (language === config.sourceLanguage) {
		throw new Error(`${language} is already the source language`);
	}
	if (config.languages.includes(language)) return config;

	await mkdir(path.join(LOCALES_DIR, language), { recursive: true });
	for (const namespace of config.namespaces) {
		const file = path.join(LOCALES_DIR, language, `${namespace}.json`);
		const source = await readNamespace(config.sourceLanguage, namespace);
		const existing = await readNamespace(language, namespace);
		await writeFile(
			file,
			serialize(normalizeTargetTree(source, existing)),
			"utf8",
		);
	}

	const next: LocaleConfig = {
		...config,
		languages: [...config.languages, language].sort(),
	};
	await writeFile(CONFIG_PATH, serialize(next), "utf8");
	return next;
}

/**
 * Where each key is used in the product. Grepping the source on demand keeps
 * the studio honest about call sites without a build step — the answer is
 * always the working tree, never a stale index.
 */
async function findUsages(
	roots: string[],
	needles: string[],
): Promise<{ file: string; line: number; text: string }[]> {
	const hits: { file: string; line: number; text: string }[] = [];
	const SKIP = new Set(["node_modules", ".next", "out", "dist", ".git"]);

	async function walk(dir: string): Promise<void> {
		if (hits.length >= 25) return;
		let entries: Dirent[];
		try {
			entries = await readdir(dir, { withFileTypes: true });
		} catch {
			return;
		}
		for (const entry of entries) {
			if (hits.length >= 25) return;
			const full = path.join(dir, entry.name);
			if (entry.isDirectory()) {
				if (!SKIP.has(entry.name)) await walk(full);
				continue;
			}
			if (!/\.(ts|tsx)$/.test(entry.name)) continue;
			const content = await readFile(full, "utf8");
			if (!needles.some((needle) => content.includes(needle))) continue;
			content.split("\n").forEach((text, index) => {
				if (hits.length >= 25) return;
				if (needles.some((needle) => text.includes(needle))) {
					hits.push({
						file: path.relative(REPO_ROOT, full),
						line: index + 1,
						text: text.trim().slice(0, 200),
					});
				}
			});
		}
	}

	for (const root of roots) await walk(root);
	return hits;
}

const REPO_ROOT = path.resolve(import.meta.dirname, "../../..");
const USAGE_ROOTS = [
	path.join(REPO_ROOT, "apps/desktop/app"),
	path.join(REPO_ROOT, "apps/desktop/components"),
	path.join(REPO_ROOT, "apps/web/app"),
	path.join(REPO_ROOT, "apps/web/components"),
	path.join(REPO_ROOT, "packages/ui/components"),
];

function json(
	res: Parameters<Connect.NextHandleFunction>[1],
	code: number,
	body: unknown,
) {
	res.statusCode = code;
	res.setHeader("Content-Type", "application/json");
	res.end(JSON.stringify(body));
}

async function readBody(req: Connect.IncomingMessage): Promise<unknown> {
	const chunks: Buffer[] = [];
	for await (const chunk of req) chunks.push(chunk as Buffer);
	return JSON.parse(Buffer.concat(chunks).toString("utf8") || "{}");
}
export function localesApi(): Plugin {
	const middleware: Connect.NextHandleFunction = async (req, res, next) => {
		const url = req.url ?? "";
		if (!url.startsWith("/api/")) return next();

		try {
			if (url === "/api/locales" && req.method === "GET") {
				return json(res, 200, await readAll());
			}

			const nsMatch = url.match(/^\/api\/locales\/([^/?#]+)\/([^/?#]+)$/);
			if (nsMatch && req.method === "PUT") {
				const [, language, namespace] = nsMatch;
				const body = await readBody(req);
				if (!isRecord(body)) throw new Error("body must be a JSON object");
				await writeNamespace(
					decodeURIComponent(language),
					decodeURIComponent(namespace),
					body,
				);
				return json(res, 200, { ok: true });
			}

			if (url === "/api/languages" && req.method === "POST") {
				const body = (await readBody(req)) as { language?: string };
				if (!body.language) throw new Error("language is required");
				return json(res, 200, await addLanguage(body.language));
			}

			if (url.startsWith("/api/usages?") && req.method === "GET") {
				const requestUrl = new URL(url, "http://translation.local");
				const keys = requestUrl.searchParams.getAll("key").filter(Boolean);
				if (keys.length === 0) throw new Error("key is required");
				return json(res, 200, await findUsages(USAGE_ROOTS, keys));
			}

			return json(res, 404, { error: `No route for ${req.method} ${url}` });
		} catch (error) {
			return json(res, 400, {
				error: error instanceof Error ? error.message : String(error),
			});
		}
	};

	return {
		name: "flow-like-locales-api",
		configureServer(server: ViteDevServer) {
			server.middlewares.use(middleware);
		},
		// `vite preview` serves the built bundle; without this the preview build
		// would render a shell that can never load a file.
		configurePreviewServer(server: PreviewServer) {
			server.middlewares.use(middleware);
		},
	};
}
