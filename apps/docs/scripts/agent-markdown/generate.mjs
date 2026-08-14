/**
 * Emits a Markdown twin next to every built docs page so agents can request
 * `Accept: text/markdown` (or append `.md`) and skip the Starlight chrome.
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

	await writeFile(htmlFile.replace(/\.html$/, ".md"), markdown, "utf8");
	return markdown.length;
}

const started = Date.now();
let pages = 0;
let bytes = 0;

for await (const htmlFile of walk(DIST_DIR)) {
	bytes += await convert(htmlFile);
	pages += 1;
}

console.log(
	`agent markdown: ${pages} pages, ${(bytes / 1024 / 1024).toFixed(1)} MB in ${(
		(Date.now() - started) / 1000
	).toFixed(1)}s`,
);
