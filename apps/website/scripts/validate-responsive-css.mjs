import { readFile } from "node:fs/promises";

const clientDirectory = new URL("../dist/client/", import.meta.url);
const homepageFiles = [
	"index.html",
	"de/index.html",
	"es/index.html",
	"fr/index.html",
	"it/index.html",
	"ja/index.html",
	"ko/index.html",
	"nl/index.html",
	"pt/index.html",
	"sv/index.html",
	"zh/index.html",
];

const stylesheetPaths = new Set();

for (const homepageFile of homepageFiles) {
	const html = await readFile(new URL(homepageFile, clientDirectory), "utf8");
	const linkTags = html.match(/<link\b[^>]*>/gi) ?? [];

	for (const linkTag of linkTags) {
		if (!/\brel=(?:["']stylesheet["']|stylesheet)(?:\s|>)/i.test(linkTag)) {
			continue;
		}

		const href = linkTag.match(/\bhref=(?:"([^"]+)"|'([^']+)'|([^\s>]+))/i);
		const path = href?.[1] ?? href?.[2] ?? href?.[3];
		if (path && /^\/_astro(?:-[a-z0-9]+)?\/[^/?#]+\.css$/i.test(path)) {
			stylesheetPaths.add(path);
		}
	}
}

if (stylesheetPaths.size === 0) {
	throw new Error("Responsive CSS validation found no homepage stylesheets.");
}

const stylesheets = await Promise.all(
	[...stylesheetPaths].map((path) =>
		readFile(new URL(`.${path}`, clientDirectory), "utf8"),
	),
);
const css = stylesheets.join("\n");

function mediaBlocks(source) {
	const blocks = [];
	let searchFrom = 0;

	while (searchFrom < source.length) {
		const start = source.indexOf("@media", searchFrom);
		if (start === -1) break;

		const openingBrace = source.indexOf("{", start);
		if (openingBrace === -1) break;

		let depth = 1;
		let cursor = openingBrace + 1;
		while (cursor < source.length && depth > 0) {
			if (source[cursor] === "{") depth += 1;
			if (source[cursor] === "}") depth -= 1;
			cursor += 1;
		}

		if (depth !== 0) {
			throw new Error(
				"Responsive CSS validation found an unterminated media rule.",
			);
		}

		blocks.push({
			query: source.slice(start + "@media".length, openingBrace).trim(),
			body: source.slice(openingBrace + 1, cursor - 1),
		});
		searchFrom = cursor;
	}

	return blocks;
}

const viewportBlocks = mediaBlocks(css).filter(({ query }) =>
	/(?:(?:min|max)-width|width\s*[<>]=?)/.test(query),
);
const breakpointChecks = [
	{
		label: "desktop navigation",
		query: /(?:width\s*>=\s*64rem|min-width\s*:\s*64rem)/,
		rule: ".lg\\:flex{display:flex}",
	},
	{
		label: "mobile navigation visibility",
		query: /(?:width\s*>=\s*64rem|min-width\s*:\s*64rem)/,
		rule: ".lg\\:hidden{display:none}",
	},
	{
		label: "footer tablet columns",
		query: /(?:width\s*>=\s*40rem|min-width\s*:\s*40rem)/,
		rule: ".sm\\:grid-cols-2{grid-template-columns:repeat(2,minmax(0,1fr))}",
	},
	{
		label: "footer desktop columns",
		query: /(?:width\s*>=\s*64rem|min-width\s*:\s*64rem)/,
		rule: ".lg\\:grid-cols-\\[1\\.4fr_repeat\\(4\\,1fr\\)\\]{grid-template-columns:1.4fr repeat(4,1fr)}",
	},
];
const missingRules = breakpointChecks.filter(
	({ query, rule }) =>
		!viewportBlocks.some(
			(block) => query.test(block.query) && block.body.includes(rule),
		),
);

const hasResponsiveV5Layout = viewportBlocks.some(
	({ query, body }) =>
		/(?:width\s*<=\s*800px|max-width\s*:\s*800px)/.test(query) &&
		/\.control-section\[[^\]]+\]\{[^}]*grid-template-columns:1fr/.test(body),
);

if (
	viewportBlocks.length === 0 ||
	missingRules.length > 0 ||
	!hasResponsiveV5Layout
) {
	const missing = missingRules.map(({ label }) => label);
	if (!hasResponsiveV5Layout) missing.push("V5 mobile layout");

	throw new Error(
		[
			"Production CSS is missing responsive homepage rules.",
			viewportBlocks.length === 0
				? "No viewport media queries were emitted."
				: "",
			missing.length > 0
				? `Missing responsive rules: ${missing.join(", ")}.`
				: "",
		]
			.filter(Boolean)
			.join(" "),
	);
}

console.log(
	`Responsive CSS validation passed for ${homepageFiles.length} homepages with ${viewportBlocks.length} viewport media queries.`,
);
