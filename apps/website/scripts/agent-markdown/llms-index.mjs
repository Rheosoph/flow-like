/**
 * Composes flow-like.com/llms.txt.
 *
 * The marketing site has no sidebar to read a table of contents from, so the
 * section layout is curated here. Anything published under an indexable route
 * that no section claims is reported at the end of the build rather than
 * silently dropped.
 */

import { markdownUrl, renderLlmsTxt } from "./llms-txt.mjs";

const SUMMARY =
	"Flow-Like is one source-available system for data, AI, automations, and apps: typed visual workflows on a Rust runtime that run on the desktop, self-hosted, or fully offline, with every action recorded.";

const DETAILS = [
	"Every page here is also served as Markdown: append `.md` to any URL (https://flow-like.com/pricing.md) or send `Accept: text/markdown`. Markdown responses carry an `x-markdown-tokens` header so agents can budget before fetching.",
	"Product pages are translated into German, Spanish, French, Italian, Japanese, Korean, Dutch, Portuguese, Swedish, and Chinese under a locale prefix (https://flow-like.com/de/pricing). English is the source of truth.",
];

/** Product pages in reading order. Their `<title>` is SEO copy, so relabel. */
const PRODUCT = [
	["/", "Flow-Like"],
	["/download/", "Download"],
	["/pricing/", "Pricing"],
	["/developers/", "For Developers"],
	["/integrations/", "Integrations & Connectors"],
	["/modern-bi/", "Business Intelligence"],
	["/security/", "Security & Compliance"],
	["/whitelabel/", "White-Label & OEM"],
];

const LEGAL = [
	["/eula/", "End-User License Agreement"],
	["/privacy-policy/", "Privacy Policy"],
	["/data-deletion/", "Data Deletion"],
	["/legal-notice/", "Legal Notice"],
	["/thirdparty/", "Third Party Licenses"],
];

/** Routes that exist but say nothing to a reader: auth hops and design previews. */
const UNINDEXABLE = [
	/^\/(callback|logout|pitch)\/$/,
	/^\/desktop\//,
	/^\/preview\//,
	/^\/thirdparty\/callback\/$/,
	// Tag pages are link lists over posts already listed below, and /blog/2..n
	// paginate the same posts.
	/^\/tags\//,
	/^\/blog\/\d+\/$/,
];

const LOCALE_PREFIX = /^\/(de|es|fr|it|ja|ko|nl|pt|sv|zh)(\/|$)/;

/** Titles carry an SEO tail (`Finance Automation | Flow-Like`); labels do not. */
function labelOf(page) {
	return page.title.split(" | ")[0].trim() || page.url;
}

function toEntry(page, label, { dated = false } = {}) {
	const published = dated && page.published ? page.published.slice(0, 10) : "";

	return {
		title: label ?? labelOf(page),
		url: markdownUrl(page.url),
		note: [published, page.description].filter(Boolean).join(" — "),
	};
}

/**
 * Blog posts are ordered and labelled by their date. Other pages carry a build
 * timestamp in the same field, so only the blog section asks for it.
 */
export function extractPublished(html) {
	const tag = /<meta\b[^>]*article:published_time[^>]*>/i.exec(html);
	if (!tag) return "";

	const content = /\bcontent\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))/i.exec(
		tag[0],
	);
	return content ? (content[1] ?? content[2] ?? content[3]).trim() : "";
}

function isIndexable(pathname) {
	if (LOCALE_PREFIX.test(pathname)) return false;
	return !UNINDEXABLE.some((pattern) => pattern.test(pathname));
}

export function buildWebsiteLlmsTxt({ pages, origin, onUnclaimed }) {
	const claimed = new Set();

	const pick = (pathname, label, options) => {
		const page = pages.get(pathname);
		if (!page) return null;
		claimed.add(pathname);
		return toEntry(page, label, options);
	};

	const list = (pathnames) =>
		pathnames.map(([pathname, label]) => pick(pathname, label)).filter(Boolean);

	/** Every indexable page directly under `prefix`, minus the prefix itself. */
	const under = (prefix, { compare, ...options } = {}) =>
		[...pages.entries()]
			.filter(
				([pathname]) =>
					pathname.startsWith(prefix) &&
					pathname !== prefix &&
					isIndexable(pathname),
			)
			.sort(compare ?? (([a], [b]) => a.localeCompare(b)))
			.map(([pathname]) => pick(pathname, undefined, options));

	const newestFirst = ([, a], [, b]) =>
		(b.published ?? "").localeCompare(a.published ?? "");

	const sections = [
		{ heading: "Product", entries: list(PRODUCT) },
		{ heading: "Industries", entries: under("/industries/") },
		{ heading: "Use Cases", entries: under("/use-cases/") },
		{
			heading: "Comparisons",
			body: "How Flow-Like relates to the tools it is usually evaluated against.",
			entries: [pick("/compare/", "Overview"), ...under("/compare/")].filter(
				Boolean,
			),
		},
		{
			heading: "Blog",
			body: "Release notes and engineering write-ups, newest first.",
			entries: [
				pick("/blog/", "Blog index"),
				...under("/blog/", { compare: newestFirst, dated: true }),
			].filter(Boolean),
		},
		{
			heading: "Optional",
			entries: [
				{
					title: "Flow-Like docs",
					url: "https://docs.flow-like.com/llms.txt",
					note: "guides, self-hosting, SDKs and the full node catalog",
				},
				{
					title: "Flow-Like web app",
					url: "https://app.flow-like.com/llms.txt",
					note: "the hosted app itself",
				},
				{
					title: "App & package store",
					url: `${origin}/store/`,
					note: "published apps and WASM node packages, indexed in /sitemap-store.xml",
				},
				...list(LEGAL),
				{
					title: "GitHub",
					url: "https://github.com/Rheosoph/flow-like",
					note: "source, issues and release notes",
				},
			],
		},
	];

	const unclaimed = [...pages.keys()]
		.filter((pathname) => isIndexable(pathname) && !claimed.has(pathname))
		.sort();
	if (unclaimed.length) onUnclaimed?.(unclaimed);

	return renderLlmsTxt({
		title: "Flow-Like",
		summary: SUMMARY,
		details: DETAILS,
		sections,
	});
}
