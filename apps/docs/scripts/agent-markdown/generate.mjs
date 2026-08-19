/**
 * Emits a Markdown twin next to every built docs page so agents can request
 * `Accept: text/markdown` (or append `.md`) and skip the Starlight chrome, plus
 * the /llms.txt index that points them at those twins.
 *
 * Runs after `astro build`; see functions/_middleware.ts for the request-time
 * half.
 */

import { readFile, readdir, writeFile } from "node:fs/promises";
import { dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";
import {
	DEFAULT_NOISE_SELECTORS,
	extractContentHtml,
	extractPageMeta,
	frontmatter,
	htmlToMarkdown,
} from "./html-to-markdown.mjs";
import { buildDocsLlmsTxt, extractSidebarOutline } from "./llms-index.mjs";

const SITE_ORIGIN = "https://docs.flow-like.com";
const DIST_DIR = resolve(
	dirname(fileURLToPath(import.meta.url)),
	"..",
	"..",
	"dist",
);
const SKIPPED_DIRS = new Set(["_astro", "pagefind"]);
const NOISE_SELECTORS = [
	...DEFAULT_NOISE_SELECTORS,
	// Starlight renders the edit link, timestamp and prev/next pager inside
	// <main>, and decorates every heading with an anchor link.
	"footer",
	".sl-anchor-link",
	".pagination-links",
];

async function* walk(dir) {
	for (const entry of await readdir(dir, { withFileTypes: true })) {
		if (entry.isDirectory()) {
			if (SKIPPED_DIRS.has(entry.name)) continue;
			yield* walk(join(dir, entry.name));
			continue;
		}
		if (entry.isFile() && entry.name.endsWith(".html")) {
			yield join(dir, entry.name);
		}
	}
}

function pageUrl(htmlFile) {
	const rel = relative(DIST_DIR, htmlFile).split(sep).join("/");
	const route = rel.endsWith("index.html")
		? `/${rel.slice(0, -"index.html".length)}`
		: `/${rel.replace(/\.html$/, "")}`;
	return new URL(route, SITE_ORIGIN).toString();
}

const pages = new Map();
let sidebar = null;

async function convert(htmlFile) {
	const html = await readFile(htmlFile, "utf8");
	const meta = extractPageMeta(html);
	const url = meta.canonical || pageUrl(htmlFile);
	const body = htmlToMarkdown(extractContentHtml(html), {
		baseUrl: url,
		noiseSelectors: NOISE_SELECTORS,
	});

	const markdown = `${frontmatter({
		title: meta.title,
		description: meta.description,
		url,
		language: meta.lang,
	})}${body}\n`;

	pages.set(new URL(url).pathname, { ...meta, url });
	sidebar ??= extractSidebarOutline(html);

	await writeFile(htmlFile.replace(/\.html$/, ".md"), markdown, "utf8");
	return markdown.length;
}

const started = Date.now();
let pageCount = 0;
let bytes = 0;

for await (const htmlFile of walk(DIST_DIR)) {
	bytes += await convert(htmlFile);
	pageCount += 1;
}

console.log(
	`agent markdown: ${pageCount} pages, ${(bytes / 1024 / 1024).toFixed(1)} MB in ${(
		(Date.now() - started) / 1000
	).toFixed(1)}s`,
);

if (!sidebar) {
	throw new Error(
		"llms.txt: no Starlight sidebar found in any built page — the outline it indexes is gone",
	);
}

const llms = buildDocsLlmsTxt({
	outline: sidebar,
	pages,
	origin: SITE_ORIGIN,
});
await writeFile(join(DIST_DIR, "llms.txt"), llms, "utf8");

console.log(`llms.txt: ${(llms.length / 1024).toFixed(1)} KB`);
