/**
 * Emits a Markdown twin next to every built HTML page so agents can request
 * `Accept: text/markdown` (or append `.md`) and skip the marketing markup, plus
 * the /llms.txt index that points them at those twins.
 *
 * Runs after `astro build`; see scripts/agent-markdown/markdown-negotiation.mjs
 * for the request-time half.
 */

import { readFile, readdir, writeFile } from "node:fs/promises";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import {
	extractContentHtml,
	extractPageMeta,
	frontmatter,
	htmlToMarkdown,
} from "./html-to-markdown.mjs";
import { buildWebsiteLlmsTxt, extractPublished } from "./llms-index.mjs";

const SITE_ORIGIN = "https://flow-like.com";
const CLIENT_DIR = resolve(
	dirname(fileURLToPath(import.meta.url)),
	"..",
	"..",
	"dist",
	"client",
);
const SKIPPED_DIRS = new Set(["pagefind"]);

function shouldSkipDirectory(name) {
	return name.startsWith("_astro") || SKIPPED_DIRS.has(name);
}

async function* walk(dir) {
	for (const entry of await readdir(dir, { withFileTypes: true })) {
		if (entry.isDirectory()) {
			if (shouldSkipDirectory(entry.name)) continue;
			yield* walk(join(dir, entry.name));
			continue;
		}
		if (entry.isFile() && entry.name.endsWith(".html")) {
			yield join(dir, entry.name);
		}
	}
}

function pageUrl(htmlFile) {
	const rel = relative(CLIENT_DIR, htmlFile).split(sep).join("/");
	const route = rel.endsWith("index.html")
		? `/${rel.slice(0, -"index.html".length)}`
		: `/${rel.replace(/\.html$/, "")}`;
	return new URL(route, SITE_ORIGIN).toString();
}

const pages = new Map();

async function convert(htmlFile) {
	const html = await readFile(htmlFile, "utf8");
	const meta = extractPageMeta(html);
	const url = meta.canonical || pageUrl(htmlFile);
	const body = htmlToMarkdown(extractContentHtml(html), { baseUrl: url });

	const markdown = `${frontmatter({
		title: meta.title,
		description: meta.description,
		url,
		language: meta.lang,
	})}${body}\n`;

	pages.set(new URL(url).pathname, {
		...meta,
		url,
		published: extractPublished(html),
	});

	await writeFile(htmlFile.replace(/\.html$/, ".md"), markdown, "utf8");
	return markdown.length;
}

const started = Date.now();
let pageCount = 0;
let bytes = 0;

for await (const htmlFile of walk(CLIENT_DIR)) {
	bytes += await convert(htmlFile);
	pageCount += 1;
}

console.log(
	`agent markdown: ${pageCount} pages, ${(bytes / 1024 / 1024).toFixed(1)} MB in ${(
		(Date.now() - started) / 1000
	).toFixed(1)}s`,
);

const llms = buildWebsiteLlmsTxt({
	pages,
	origin: SITE_ORIGIN,
	onUnclaimed: (routes) =>
		console.warn(
			`llms.txt: ${routes.length} route(s) match no section and are missing from the index:\n  ${routes.join("\n  ")}`,
		),
});
await writeFile(join(CLIENT_DIR, "llms.txt"), llms, "utf8");

console.log(`llms.txt: ${(llms.length / 1024).toFixed(1)} KB`);
