/**
 * Composes docs.flow-like.com/llms.txt.
 *
 * The Starlight sidebar is the curated table of contents for this site, and the
 * build renders it into every page, so the outline is read back out of the HTML
 * instead of being maintained a second time here.
 */

import * as cheerio from "cheerio";
import { markdownUrl, renderLlmsTxt } from "./llms-txt.mjs";

const SUMMARY =
	"Flow-Like is a source-available platform for building typed, visual workflows — drag-and-drop nodes with typed pins on a Rust runtime — that run on the desktop, self-hosted, or fully offline.";

const DETAILS = [
	"Every page here is also served as Markdown: append `.md` to any URL (https://docs.flow-like.com/start/getting-started.md) or send `Accept: text/markdown`. Markdown responses carry an `x-markdown-tokens` header so agents can budget before fetching.",
	"Sections follow the documentation sidebar. Nested groups are indented; a bold entry is a group heading rather than a page.",
];

const NODE_CATALOG_HEADING = "Node Catalog";
const NODE_CATALOG_BODY =
	"One generated reference page per built-in node — pins, schemas, defaults and risk ratings. Only the categories are listed here because each category page links every node it contains.";

/** Starlight puts the label of a `<summary>` inside `.group-label > .large`. */
function groupLabel($, details) {
	return $(details).children("summary").find(".large").first().text();
}

/** Sidebar links carry an optional badge span the label must not swallow. */
function linkLabel($, anchor) {
	const label = $(anchor).children("span").not(".sl-badge").first().text();
	return label || $(anchor).text();
}

function readItems($, list) {
	const items = [];

	for (const li of $(list).children("li").toArray()) {
		const details = $(li).children("details").first();
		if (details.length) {
			items.push({
				label: groupLabel($, details),
				items: readItems($, details.children("ul").first()),
			});
			continue;
		}

		const anchor = $(li).children("a[href]").first();
		if (anchor.length) {
			items.push({ label: linkLabel($, anchor), href: anchor.attr("href") });
		}
	}

	return items;
}

/**
 * @returns {Array<{label: string, items?: object[], href?: string}> | null} the
 * sidebar tree, or null for pages that render without one.
 */
export function extractSidebarOutline(html) {
	const $ = cheerio.load(html);
	const top = $("#starlight__sidebar ul.top-level").first();
	if (!top.length) return null;

	const outline = readItems($, top);
	return outline.length ? outline : null;
}

function entry(label, href, { pages, origin, depth = 0 }) {
	const url = new URL(href, origin).toString();
	const page = pages.get(new URL(url).pathname);

	return {
		title: label,
		url: markdownUrl(page?.url ?? url),
		note: page?.description,
		depth,
	};
}

function toEntries(items, context, depth = 0) {
	const entries = [];

	for (const item of items) {
		if (item.items) {
			entries.push({ title: item.label, depth });
			entries.push(...toEntries(item.items, context, depth + 1));
			continue;
		}
		entries.push(entry(item.label, item.href, { ...context, depth }));
	}

	return entries;
}

/**
 * 1400+ node pages would bury the rest of the index, and their category pages
 * already list them, so the catalog collapses to one line per category.
 */
function toCategoryEntries(items, context) {
	return items
		.map((item) => {
			const href = item.items
				? item.items.find((child) => child.href)?.href
				: item.href;
			return href ? entry(item.label, href, context) : null;
		})
		.filter(Boolean);
}

/** A group that only wraps other groups reads better without the empty bullet. */
function dropEmptyGroups(entries) {
	return entries.filter((current, index) => {
		if (current.url) return true;
		const next = entries[index + 1];
		return next !== undefined && next.depth > current.depth;
	});
}

/**
 * @param {object} input
 * @param {Array} input.outline result of extractSidebarOutline
 * @param {Map<string, {url: string, description: string}>} input.pages by pathname
 * @param {string} input.origin
 */
export function buildDocsLlmsTxt({ outline, pages, origin }) {
	const context = { pages, origin };

	const sections = outline.map((group) => {
		const isNodeCatalog = group.label === NODE_CATALOG_HEADING;

		return {
			heading: group.label,
			body: isNodeCatalog ? NODE_CATALOG_BODY : undefined,
			entries: isNodeCatalog
				? toCategoryEntries(group.items, context)
				: dropEmptyGroups(toEntries(group.items, context)),
		};
	});

	sections.push({
		heading: "Optional",
		entries: [
			{
				title: "Flow-Like website",
				url: "https://flow-like.com/llms.txt",
				note: "product pages, pricing, comparisons and the engineering blog",
			},
			{
				title: "Flow-Like web app",
				url: "https://app.flow-like.com/llms.txt",
				note: "the hosted app itself",
			},
			{
				title: "Sitemap",
				url: `${origin}/sitemap-index.xml`,
				note: "every page, including all node references",
			},
			{
				title: "GitHub",
				url: "https://github.com/Rheosoph/flow-like",
				note: "source, issues and release notes",
			},
		],
	});

	return renderLlmsTxt({
		title: "Flow-Like Docs",
		summary: SUMMARY,
		details: DETAILS,
		sections,
	});
}
