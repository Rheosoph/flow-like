import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { load } from "cheerio";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../dist/client");
const site = new URL("https://flow-like.com");
const failures = [];
const check = (condition, route, message) => {
	if (!condition) failures.push(`${route}: ${message}`);
};
const read = (file) => readFileSync(file, "utf8");
const isFile = (file) => existsSync(file) && statSync(file).isFile();
const walk = (dir) => existsSync(dir)
	? readdirSync(dir, { withFileTypes: true }).flatMap((entry) =>
		entry.isDirectory() ? walk(join(dir, entry.name)) : [join(dir, entry.name)])
	: [];
const url = (value, base) => {
	try { return value ? new URL(value, base) : undefined; } catch { return undefined; }
};
const localFile = (target) => join(root, decodeURIComponent(target.pathname));
const localTargetExists = (target) => isFile(localFile(target)) || isFile(join(localFile(target), "index.html"));
const https = (value) => url(value)?.protocol === "https:";
const types = (node, type) => [node?.["@type"]].flat().includes(type);
const flatten = (value) => Array.isArray(value) ? value.flatMap(flatten)
	: value && typeof value === "object" ? [value, ...flatten(value["@graph"])] : [];
const imageUrl = (value) => typeof value === "string" ? value : value?.url ?? value?.contentUrl;

// Apply the most specific Googlebot group, then the longest matching path rule.
const robotGroups = [];
let group;
for (const line of existsSync(join(root, "robots.txt")) ? read(join(root, "robots.txt")).split(/\r?\n/) : []) {
	const match = line.replace(/#.*/, "").match(/^\s*(user-agent|allow|disallow)\s*:\s*(.*?)\s*$/i);
	if (!match) continue;
	const [, rawKey, value] = match;
	const key = rawKey.toLowerCase();
	if (key === "user-agent") {
		if (!group || group.rules.length) { group = { agents: [], rules: [] }; robotGroups.push(group); }
		group.agents.push(value.toLowerCase());
	} else if (group && value) group.rules.push({ allow: key === "allow", path: value });
}
const specific = robotGroups.filter((item) => item.agents.includes("googlebot"));
const robotRules = (specific.length ? specific : robotGroups.filter((item) => item.agents.includes("*")))
	.flatMap((item) => item.rules).map((rule) => ({ ...rule,
		pattern: new RegExp(`^${rule.path.replace(/[.+?^${}()|[\]\\]/g, "\\$&").replace(/\*/g, ".*").replace(/\\\$$/, "$")}`),
		length: rule.path.replace(/[*$]/g, "").length,
	}));

const files = [...walk(join(root, "blog")), ...walk(join(root, "tags"))].filter((file) => file.endsWith(".html"));
check(files.length > 0, "build", "no blog or tag HTML found; build the website first");
const titles = new Map();
const descriptions = new Map();
const articles = [];
for (const file of files) {
	const route = `/${relative(root, file).replace(/index\.html$/, "")}`;
	const pageUrl = new URL(route, site);
	const $ = load(read(file));
	for (const [label, selector, values] of [
		["title", "head > title", titles], ["description", 'meta[name="description"]', descriptions],
	]) {
		const nodes = $(selector);
		const value = (label === "title" ? nodes.text() : nodes.attr("content") ?? "").trim().replace(/\s+/g, " ");
		check(nodes.length === 1 && value, route, `expected one nonempty ${label}`);
		check(!values.has(value), route, `duplicate ${label} (also ${values.get(value)})`);
		if (value) values.set(value, route);
	}
	const canonicals = $('link[rel="canonical"]');
	const canonical = url(canonicals.attr("href"));
	check(canonicals.length === 1 && canonical?.protocol === "https:", route, "expected one absolute HTTPS canonical");
	if (canonical?.origin === site.origin) check(canonical.href === pageUrl.href && route.endsWith("/"), route, "canonical must match this route with a trailing slash");
	const robots = $('meta[name="robots"], meta[name="googlebot"]').map((_, node) => $(node).attr("content") ?? "").get().join(",");
	check(!/(?:^|[\s,])(noindex|none)(?:$|[\s,])/i.test(robots), route, "page blocks indexing in robots metadata");
	const matchingRules = robotRules.filter((rule) => rule.pattern.test(route)).sort((a, b) => b.length - a.length || Number(b.allow) - Number(a.allow));
	check(matchingRules[0]?.allow !== false, route, "robots.txt blocks Googlebot");
	check($("h1").length === 1, route, `expected one H1, found ${$("h1").length}`);
	const structured = [];
	$('script[type="application/ld+json"]').each((_, node) => {
		try { structured.push(...flatten(JSON.parse($(node).text()))); }
		catch { check(false, route, "invalid JSON-LD"); }
	});
	const article = /^\/blog\/[^/]+\/$/.test(route) && !/^\/blog\/\d+\/$/.test(route);
	if (article) {
		articles.push(pageUrl.href);
		const post = structured.find((node) => types(node, "BlogPosting"));
		check(Boolean(post), route, "missing BlogPosting structured data");
		if (post) {
			check(typeof post.headline === "string" && post.headline.trim(), route, "BlogPosting needs a headline");
			for (const field of ["datePublished", "dateModified"]) check(typeof post[field] === "string" && Number.isFinite(Date.parse(post[field])), route, `BlogPosting needs a valid ${field}`);
			check([post.author].flat().some((author) => author?.name && https(author.url)), route, "BlogPosting needs an author name and HTTPS URL");
			check([post.image].flat().some((image) => https(imageUrl(image))), route, "BlogPosting needs an absolute HTTPS image URL");
		}
		check(structured.some((node) => types(node, "BreadcrumbList") && node.itemListElement?.length >= 2), route, "missing BreadcrumbList structured data");
	}
	$('a[href]').each((_, node) => {
		const target = url($(node).attr("href"), pageUrl);
		if (target?.origin === site.origin && /^\/(blog|tags)(\/|$)/.test(target.pathname)) check(localTargetExists(target), route, `broken internal link ${target.pathname}`);
	});
	const checkImage = (value) => {
		const target = url(value, pageUrl);
		if (target?.origin === site.origin) check(isFile(localFile(target)), route, `missing image ${target.pathname}`);
	};
	$('img[src], meta[property="og:image"], meta[name="twitter:image"]').each((_, node) => checkImage($(node).attr("src") ?? $(node).attr("content")));
	for (const item of structured) for (const image of [item.image].flat()) checkImage(imageUrl(image));
	$('img[srcset], source[srcset]').each((_, node) => {
		for (const candidate of $(node).attr("srcset").split(",")) checkImage(candidate.trim().split(/\s+/)[0]);
	});
	const covers = $('img[data-blog-cover]');
	if (route !== "/tags/") check(covers.length > 0, route, "missing responsive blog cover");
	covers.each((_, node) => {
		const cover = $(node);
		check(Number(cover.attr("width")) > 0 && Number(cover.attr("height")) > 0, route, "cover needs intrinsic width and height");
		const target = url(cover.attr("src"), pageUrl);
		const generated = target?.origin === site.origin && /^\/_astro(?:-[^/]+)?\//.test(target.pathname);
		check(!(target?.origin === site.origin && /^\/images\/[^/]+\.(jpe?g|png|webp|avif)$/i.test(target.pathname)), route, "local cover must use generated responsive assets");
		if (!generated) return;
		check(/\.webp$/i.test(target.pathname), route, "generated cover source must be WebP");
		const candidates = (cover.attr("srcset") ?? "").split(",").filter((value) => value.trim());
		check(candidates.length >= 2 && Boolean(cover.attr("sizes")), route, "cover needs responsive srcset and sizes");
		check(candidates.every((candidate) => /\.webp\s+\d+w$/i.test(candidate.trim())), route, "cover srcset must contain WebP width candidates");
	});
}

const rssFile = join(root, "rss.xml");
check(isFile(rssFile), "feed", "missing rss.xml");
const rss = isFile(rssFile) ? load(read(rssFile), { xml: true }) : undefined;
const feedUrls = new Set(rss ? rss("item > link").map((_, node) => rss(node).text().trim()).get() : []);
const sitemapUrls = new Set();
for (const file of walk(root).filter((file) => /\/sitemap[^/]*\.xml$/.test(file))) {
	const xml = load(read(file), { xml: true });
	xml("url > loc").each((_, node) => sitemapUrls.add(xml(node).text().trim()));
}
check(articles.length > 0, "build", "no blog articles found");
for (const article of articles) {
	check(feedUrls.has(article), article, "missing from RSS");
	check(sitemapUrls.has(article), article, "missing from sitemap");
}
if (failures.length) {
	console.error(`Blog SEO validation failed (${failures.length} issues):\n${failures.map((failure) => `- ${failure}`).join("\n")}`);
	process.exitCode = 1;
} else console.log(`Blog SEO validation passed: ${articles.length} articles, ${files.length - articles.length} archive/tag pages; metadata, links, images, RSS, and sitemap checked.`);
