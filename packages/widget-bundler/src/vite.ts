import { existsSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import type { HtmlTagDescriptor, Plugin } from "vite";
import { type ExtractResult, extractContract } from "./extract";
import { contractScriptContent } from "./inline";

export interface WidgetEntry {
	id: string;
	htmlPath: string;
}

/** Find `src/widgets/{id}/index.html` entrypoints of a framework group. */
export function discoverWidgetEntries(rootDir: string): WidgetEntry[] {
	const widgetsDir = join(rootDir, "src", "widgets");
	if (!existsSync(widgetsDir)) return [];
	const entries: WidgetEntry[] = [];
	for (const name of readdirSync(widgetsDir).sort()) {
		const dir = join(widgetsDir, name);
		if (!statSync(dir).isDirectory()) continue;
		const htmlPath = join(dir, "index.html");
		if (existsSync(htmlPath)) entries.push({ id: name, htmlPath });
	}
	return entries;
}

export function widgetIdFromHtmlPath(
	rootDir: string,
	filePath: string,
): string | null {
	const rel = relative(resolve(rootDir), resolve(filePath))
		.split(sep)
		.join("/");
	const match = /^src\/widgets\/([^/]+)\/index\.html$/.exec(rel);
	return match?.[1] ?? null;
}

const extractCache = new Map<
	string,
	{ mtimeMs: number; result: ExtractResult }
>();

function cachedExtract(configPath: string): ExtractResult {
	const mtimeMs = statSync(configPath).mtimeMs;
	const cached = extractCache.get(configPath);
	if (cached && cached.mtimeMs === mtimeMs) return cached.result;
	const result = extractContract(configPath);
	extractCache.set(configPath, { mtimeMs, result });
	return result;
}

/**
 * Contract script tag for a widget `index.html`, or `null` when the page is
 * not a widget entry (dev parity with `pack`, which re-injects the contract).
 */
export function contractTagForHtml(
	rootDir: string,
	htmlFilePath: string,
): HtmlTagDescriptor | null {
	const id = widgetIdFromHtmlPath(rootDir, htmlFilePath);
	if (!id) return null;
	const configPath = join(dirname(resolve(htmlFilePath)), "widget.config.ts");
	if (!existsSync(configPath)) return null;
	const { contract } = cachedExtract(configPath);
	return {
		tag: "script",
		children: contractScriptContent(contract),
		injectTo: "head-prepend",
	};
}

/**
 * Vite plugin for widget framework groups: discovers `src/widgets/{id}/index.html`
 * as build inputs (one entry per widget id), routes entry/chunk/asset output
 * into `shared/` so `pack` can collect deduplicated chunks, and injects the
 * extracted `__FLW_CONTRACT__` script during dev/build.
 */
export function flowLikeWidgets(): Plugin {
	let root = process.cwd();
	return {
		name: "flow-like-widgets",
		config(userConfig) {
			root = resolve(userConfig.root ?? process.cwd());
			const entries = discoverWidgetEntries(root);
			if (entries.length === 0) return {};
			return {
				build: {
					rollupOptions: {
						input: Object.fromEntries(
							entries.map((entry) => [entry.id, entry.htmlPath]),
						),
						output: {
							entryFileNames: "shared/[name]-[hash].js",
							chunkFileNames: "shared/[name]-[hash].js",
							assetFileNames: "shared/[name]-[hash][extname]",
						},
					},
				},
			};
		},
		transformIndexHtml: {
			order: "pre",
			handler(html, ctx) {
				const tag = contractTagForHtml(root, ctx.filename);
				if (!tag) return;
				return { html, tags: [tag] };
			},
		},
	};
}
