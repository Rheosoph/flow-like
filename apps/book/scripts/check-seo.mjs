import { access, readFile, readdir } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

const appRoot = fileURLToPath(new URL("..", import.meta.url));
const distRoot = path.join(appRoot, "dist");
const origin = "https://book.flow-like.com";
const failures = [];

function check(condition, message) {
	if (!condition) failures.push(message);
}

async function findFiles(directory, extension) {
	const entries = await readdir(directory, { withFileTypes: true });
	const files = await Promise.all(
		entries.map(async (entry) => {
			const absolute = path.join(directory, entry.name);
			if (entry.isDirectory()) return findFiles(absolute, extension);
			return entry.isFile() && entry.name.endsWith(extension) ? [absolute] : [];
		}),
	);
	return files.flat();
}

function parseAttributes(tag) {
	const attributes = new Map();
	for (const match of tag.matchAll(
		/([^\s=<>]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+)))?/g,
	)) {
		const [, name, doubleQuoted, singleQuoted, unquoted] = match;
		if (!name || name.startsWith("<")) continue;
		attributes.set(
			name.toLowerCase(),
			doubleQuoted ?? singleQuoted ?? unquoted ?? "",
		);
	}
	return attributes;
}

function tags(html, name) {
	return [...html.matchAll(new RegExp(`<${name}\\b[^>]*>`, "gi"))].map(
		(match) => ({ source: match[0], attrs: parseAttributes(match[0]) }),
	);
}

function selectedAttributeValues(
	html,
	tagName,
	selectorName,
	selectorValue,
	valueName,
) {
	return tags(html, tagName)
		.filter((tag) => tag.attrs.get(selectorName) === selectorValue)
		.map((tag) => tag.attrs.get(valueName))
		.filter((value) => value !== undefined);
}

function metaValues(html, selectorName, selectorValue) {
	return selectedAttributeValues(
		html,
		"meta",
		selectorName,
		selectorValue,
		"content",
	);
}

function linkValues(html, relation) {
	return selectedAttributeValues(html, "link", "rel", relation, "href");
}

function routeFromFile(file) {
	const relative = path.relative(distRoot, file).split(path.sep).join("/");
	if (relative === "index.html") return "/";
	if (relative === "404.html") return "/404/";
	return `/${relative.replace(/\/index\.html$/, "/")}`;
}

function jsonLdBlocks(html) {
	return [
		...html.matchAll(
			/<script\b[^>]*type="application\/ld\+json"[^>]*>([\s\S]*?)<\/script>/gi,
		),
	].map((match) => match[1]);
}

function graphTypes(structuredData) {
	const graph = Array.isArray(structuredData?.["@graph"])
		? structuredData["@graph"]
		: [structuredData];
	return new Set(
		graph.flatMap((node) => {
			const type = node?.["@type"];
			return Array.isArray(type) ? type : type ? [type] : [];
		}),
	);
}

function parseHeaderRules(source) {
	const rules = new Map();
	let currentRule;

	for (const line of source.split(/\r?\n/)) {
		if (!line.trim()) {
			currentRule = undefined;
			continue;
		}
		if (!/^\s/.test(line)) {
			currentRule = line.trim();
			rules.set(currentRule, new Map());
			continue;
		}
		if (!currentRule) continue;

		const separator = line.indexOf(":");
		if (separator < 0) continue;
		const name = line.slice(0, separator).trim().toLowerCase();
		const value = line.slice(separator + 1).trim();
		rules.get(currentRule)?.set(name, value);
	}

	return rules;
}

async function assertLocalUrlExists(url, route) {
	let parsed;
	try {
		parsed = new URL(url);
	} catch {
		failures.push(`${route}: invalid absolute URL ${url}`);
		return;
	}
	check(
		parsed.origin === origin,
		`${route}: asset uses unexpected origin ${url}`,
	);
	const target = path.join(
		distRoot,
		decodeURIComponent(parsed.pathname).replace(/^\//, ""),
	);
	try {
		await access(target);
	} catch {
		failures.push(`${route}: referenced asset is missing: ${parsed.pathname}`);
	}
}

const htmlFiles = await findFiles(distRoot, ".html");
const publicPages = [];
const titles = new Map();
const descriptions = new Map();

for (const file of htmlFiles) {
	const route = routeFromFile(file);
	const html = await readFile(file, "utf8");
	const head = html.split("</head>", 1)[0];
	const titleValues = [
		...head.matchAll(/<title\b[^>]*>([\s\S]*?)<\/title>/gi),
	].map((match) => match[1].trim());
	const descriptionValues = metaValues(head, "name", "description");
	const robotsValues = metaValues(head, "name", "robots");
	const canonicalValues = linkValues(head, "canonical");
	const isPrint = route === "/print/";
	const isNotFound = route === "/404/";

	check(titleValues.length === 1, `${route}: expected exactly one <title>`);
	check(
		descriptionValues.length === 1,
		`${route}: expected exactly one meta description`,
	);
	check(
		robotsValues.length === 1,
		`${route}: expected exactly one robots meta tag`,
	);
	if (!isPrint) {
		check(tags(html, "h1").length === 1, `${route}: expected exactly one h1`);
	}

	if (isPrint || isNotFound) {
		check(
			robotsValues[0]?.replaceAll(" ", "").startsWith("noindex,"),
			`${route}: non-indexable page must declare noindex`,
		);
		if (isNotFound) {
			check(
				canonicalValues.length === 0,
				`${route}: 404 must not emit a canonical`,
			);
			check(
				metaValues(head, "property", "og:type").length === 0,
				`${route}: 404 must not emit Open Graph metadata`,
			);
			check(
				jsonLdBlocks(head).length === 0,
				`${route}: 404 must not emit JSON-LD`,
			);
		}
		continue;
	}

	publicPages.push({ route, html, head });
	check(
		!robotsValues[0]?.includes("noindex"),
		`${route}: public page is noindex`,
	);
	check(
		canonicalValues.length === 1,
		`${route}: expected exactly one canonical`,
	);
	const expectedCanonical = new URL(route, origin).toString();
	check(
		canonicalValues[0] === expectedCanonical,
		`${route}: canonical mismatch (${canonicalValues[0]} != ${expectedCanonical})`,
	);

	const metadata = [
		["property", "og:type"],
		["property", "og:site_name"],
		["property", "og:url"],
		["property", "og:title"],
		["property", "og:description"],
		["property", "og:image"],
		["property", "og:image:width"],
		["property", "og:image:height"],
		["property", "og:image:alt"],
		["name", "twitter:card"],
		["name", "twitter:title"],
		["name", "twitter:description"],
		["name", "twitter:image"],
		["name", "twitter:image:alt"],
	];
	for (const [selector, name] of metadata) {
		check(
			metaValues(head, selector, name).length === 1,
			`${route}: expected exactly one ${name}`,
		);
	}

	check(
		metaValues(head, "property", "og:url")[0] === expectedCanonical,
		`${route}: og:url does not match canonical`,
	);
	const ogImage = metaValues(head, "property", "og:image")[0];
	const twitterImage = metaValues(head, "name", "twitter:image")[0];
	check(
		ogImage === twitterImage,
		`${route}: Open Graph and Twitter images differ`,
	);
	if (ogImage) await assertLocalUrlExists(ogImage, route);

	const blocks = jsonLdBlocks(head);
	check(blocks.length === 1, `${route}: expected exactly one JSON-LD graph`);
	if (blocks.length === 1) {
		try {
			const structuredData = JSON.parse(blocks[0]);
			check(
				structuredData["@context"] === "https://schema.org",
				`${route}: unexpected JSON-LD context`,
			);
			const types = graphTypes(structuredData);
			for (const required of [
				"Organization",
				"WebSite",
				"Book",
				"ImageObject",
			]) {
				check(types.has(required), `${route}: JSON-LD is missing ${required}`);
			}
			if (route === "/") {
				check(types.has("WebPage"), `${route}: JSON-LD is missing WebPage`);
			} else if (route === "/contents/" || /^\/part-\d+\/$/.test(route)) {
				check(
					types.has("CollectionPage"),
					`${route}: JSON-LD is missing CollectionPage`,
				);
				check(types.has("ItemList"), `${route}: JSON-LD is missing ItemList`);
				check(
					types.has("BreadcrumbList"),
					`${route}: JSON-LD is missing BreadcrumbList`,
				);
			} else {
				check(types.has("Chapter"), `${route}: JSON-LD is missing Chapter`);
				check(
					types.has("BreadcrumbList"),
					`${route}: JSON-LD is missing BreadcrumbList`,
				);
			}
		} catch (error) {
			failures.push(`${route}: JSON-LD does not parse (${error.message})`);
		}
	}

	const title = titleValues[0];
	const description = descriptionValues[0];
	if (title) {
		check(
			!titles.has(title),
			`${route}: duplicate title also used by ${titles.get(title)}`,
		);
		titles.set(title, route);
	}
	if (description) {
		check(
			!descriptions.has(description),
			`${route}: duplicate description also used by ${descriptions.get(description)}`,
		);
		descriptions.set(description, route);
	}

	check(
		metaValues(head, "name", "keywords").length === 0,
		`${route}: obsolete meta keywords tag found`,
	);
	check(
		!head.includes("SearchAction"),
		`${route}: removed sitelinks SearchAction found`,
	);

	if (html.includes("Release check:")) {
		check(
			/<(?:div|span|section)\b[^>]*data-nosnippet[^>]*>[\s\S]*?<blockquote\b[^>]*>[\s\S]*?Release check:/.test(
				html,
			),
			`${route}: release check is eligible for search snippets`,
		);
		check(
			!/<blockquote\b[^>]*data-nosnippet/i.test(html),
			`${route}: data-nosnippet uses an unsupported blockquote host`,
		);
	}
}

const sitemap = await readFile(path.join(distRoot, "sitemap-0.xml"), "utf8");
const sitemapUrls = new Set(
	[...sitemap.matchAll(/<loc>([^<]+)<\/loc>/g)].map((match) => match[1]),
);
const canonicalUrls = new Set(
	publicPages.map(({ route }) => new URL(route, origin).toString()),
);
check(
	!sitemapUrls.has(`${origin}/print/`),
	"sitemap: /print/ must be excluded",
);
check(!sitemapUrls.has(`${origin}/404/`), "sitemap: /404/ must be excluded");
for (const canonical of canonicalUrls) {
	check(sitemapUrls.has(canonical), `sitemap: missing canonical ${canonical}`);
}
for (const sitemapUrl of sitemapUrls) {
	check(canonicalUrls.has(sitemapUrl), `sitemap: unexpected URL ${sitemapUrl}`);
}

const robots = await readFile(path.join(distRoot, "robots.txt"), "utf8");
check(
	robots.includes("User-agent: *"),
	"robots.txt: missing wildcard user agent",
);
check(robots.includes("Allow: /"), "robots.txt: missing root allow rule");
check(
	robots.includes(`Sitemap: ${origin}/sitemap-index.xml`),
	"robots.txt: missing absolute sitemap index URL",
);
check(
	!/Disallow:\s*\/print\//.test(robots),
	"robots.txt: /print/ must remain crawlable for noindex",
);

const headers = await readFile(path.join(distRoot, "_headers"), "utf8");
const headerRules = parseHeaderRules(headers);
const pdfRobotsHeader = headerRules.get("/flowbook.pdf")?.get("x-robots-tag");
const printRobotsHeader = headerRules.get("/print/*")?.get("x-robots-tag");
check(
	pdfRobotsHeader?.replaceAll(" ", "").toLowerCase() === "noindex,follow",
	"_headers: /flowbook.pdf must set X-Robots-Tag: noindex, follow",
);
check(
	printRobotsHeader?.replaceAll(" ", "").toLowerCase() === "noindex,follow",
	"_headers: /print/* must set X-Robots-Tag: noindex, follow",
);

const manifest = JSON.parse(
	await readFile(path.join(distRoot, "site.webmanifest"), "utf8"),
);
check(
	manifest.name === "FlowBook: A Developer's Guide to Flow-Like",
	"manifest: unexpected app name",
);
await access(path.join(distRoot, "favicon.svg"));
await access(path.join(distRoot, "flowbook.pdf"));

if (failures.length > 0) {
	console.error(`SEO validation failed with ${failures.length} issue(s):`);
	for (const failure of failures) console.error(`- ${failure}`);
	process.exit(1);
}

console.log(
	`SEO validation passed for ${publicPages.length} indexable pages, ${sitemapUrls.size} sitemap URLs, and ${htmlFiles.length} HTML documents.`,
);
