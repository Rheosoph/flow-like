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

function headLinks(html) {
	const head = html.split("</head>", 1)[0];
	return [...head.matchAll(/<link\b[^>]*>/gi)].map((match) =>
		parseAttributes(match[0]),
	);
}

function markdownPathForHtmlPath(htmlPath) {
	return htmlPath === "/" ? "/index.md" : `${htmlPath}index.md`;
}

function distFileForPath(pathname) {
	const relative = decodeURIComponent(pathname).replace(/^\//, "");
	return path.join(
		distRoot,
		pathname.endsWith("/") ? relative : "",
		pathname.endsWith("/") ? "index.html" : relative,
	);
}

function routeFromFile(file) {
	return `/${path.relative(distRoot, file).split(path.sep).join("/")}`;
}

function outsideCodeFences(markdown) {
	const output = [];
	let fenceCharacter;
	let fenceLength = 0;

	for (const line of markdown.split("\n")) {
		const marker = line.match(/^ {0,3}(`{3,}|~{3,})/);
		if (marker) {
			const character = marker[1][0];
			if (!fenceCharacter) {
				fenceCharacter = character;
				fenceLength = marker[1].length;
			} else if (
				character === fenceCharacter &&
				marker[1].length >= fenceLength
			) {
				fenceCharacter = undefined;
				fenceLength = 0;
			}
			continue;
		}
		if (!fenceCharacter) output.push(line);
	}

	return { content: output.join("\n"), balanced: !fenceCharacter };
}

function containsRawEsm(markdown) {
	return markdown.split("\n").some((line) => {
		const source = line.trim();
		return (
			/^import(?:\s+type)?\s+(?:["'{*]|[A-Za-z_$][\w$]*(?:\s*,|\s+from\s+["']))/.test(
				source,
			) ||
			/^export\s+(?:default\b|(?:const|let|var|async\s+function|function|class|type|interface|enum|namespace)\b|\{|\*)/.test(
				source,
			)
		);
	});
}

function markdownLinks(markdown) {
	const { content } = outsideCodeFences(markdown);
	return [...content.matchAll(/\[[^\]]*\]\(([^\s)]+)(?:\s+"[^"]*")?\)/g)].map(
		(match) => match[1],
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

async function checkLocalUrl(href, source) {
	let url;
	try {
		url = new URL(href, origin);
	} catch {
		failures.push(`${source}: invalid URL ${href}`);
		return;
	}
	if (url.origin !== origin) return;

	try {
		await access(distFileForPath(url.pathname));
	} catch {
		failures.push(`${source}: local link does not resolve: ${url.pathname}`);
	}
}

const sitemap = await readFile(path.join(distRoot, "sitemap-0.xml"), "utf8");
const canonicalUrls = [...sitemap.matchAll(/<loc>([^<]+)<\/loc>/g)].map(
	(match) => match[1],
);
const canonicalPaths = canonicalUrls.map((url) => new URL(url).pathname);
const expectedMarkdownPaths = new Set(
	canonicalPaths.map(markdownPathForHtmlPath),
);
const markdownFiles = await findFiles(distRoot, ".md");
const generatedMarkdownPaths = new Set(markdownFiles.map(routeFromFile));

check(
	generatedMarkdownPaths.size === expectedMarkdownPaths.size,
	`markdown: expected ${expectedMarkdownPaths.size} files, found ${generatedMarkdownPaths.size}`,
);
for (const expected of expectedMarkdownPaths) {
	check(generatedMarkdownPaths.has(expected), `markdown: missing ${expected}`);
}
for (const generated of generatedMarkdownPaths) {
	check(
		expectedMarkdownPaths.has(generated),
		`markdown: unexpected ${generated}`,
	);
}

for (const canonicalUrl of canonicalUrls) {
	const canonical = new URL(canonicalUrl);
	const html = await readFile(distFileForPath(canonical.pathname), "utf8");
	const links = headLinks(html);
	const markdownAlternates = links.filter(
		(link) =>
			link.get("rel") === "alternate" && link.get("type") === "text/markdown",
	);
	const describedBy = links.filter((link) => link.get("rel") === "describedby");
	const expectedMarkdownUrl = new URL(
		markdownPathForHtmlPath(canonical.pathname),
		origin,
	).toString();

	check(
		markdownAlternates.length === 1,
		`${canonical.pathname}: expected one Markdown alternate`,
	);
	check(
		markdownAlternates[0]?.get("href") === expectedMarkdownUrl,
		`${canonical.pathname}: Markdown alternate does not match ${expectedMarkdownUrl}`,
	);
	check(
		describedBy.length === 1 &&
			describedBy[0]?.get("href") === `${origin}/llms.txt`,
		`${canonical.pathname}: missing llms.txt describedby discovery link`,
	);
}

for (const file of markdownFiles) {
	const route = routeFromFile(file);
	const markdown = await readFile(file, "utf8");
	const outside = outsideCodeFences(markdown);
	const h1s = outside.content.match(/^#\s+\S.*$/gm) ?? [];
	const expectedHtmlPath =
		route === "/index.md" ? "/" : route.replace(/index\.md$/, "");
	const expectedCanonical = new URL(expectedHtmlPath, origin).toString();
	const expectedMarkdown = new URL(route, origin).toString();

	check(outside.balanced, `${route}: unbalanced fenced code block`);
	check(h1s.length === 1, `${route}: expected exactly one Markdown H1`);
	check(markdown.startsWith("# "), `${route}: document must begin with its H1`);
	check(
		markdown.includes(
			`- **Canonical HTML:** [${expectedCanonical}](${expectedCanonical})`,
		),
		`${route}: canonical HTML metadata is missing or incorrect`,
	);
	check(
		markdown.includes(
			`- **Markdown alternate:** [${expectedMarkdown}](${expectedMarkdown})`,
		),
		`${route}: Markdown alternate metadata is missing or incorrect`,
	);
	check(
		markdown.length > 800,
		`${route}: Markdown output is unexpectedly thin`,
	);
	check(
		!containsRawEsm(outside.content),
		`${route}: raw ESM remains in Markdown output`,
	);
	check(
		!/^\s*<\/?[A-Z][A-Za-z0-9]*(?:\s|\/?>)/m.test(outside.content),
		`${route}: raw MDX component remains in Markdown output`,
	);
	check(
		!outside.content.includes("client:"),
		`${route}: client directive leaked`,
	);
	check(
		!outside.content.includes("src/content/") &&
			!outside.content.includes(".astro") &&
			!outside.content.includes(".tsx"),
		`${route}: implementation source path leaked`,
	);

	for (const href of markdownLinks(markdown)) {
		await checkLocalUrl(href, route);
	}

	const html = await readFile(distFileForPath(expectedHtmlPath), "utf8");
	if (html.includes("Release check:")) {
		check(
			markdown.includes("Release check:"),
			`${route}: release-check caveat was lost during conversion`,
		);
	}
}

const llmsPath = path.join(distRoot, "llms.txt");
const llmsFullPath = path.join(distRoot, "llms-full.txt");
const llms = await readFile(llmsPath, "utf8");
const llmsFull = await readFile(llmsFullPath, "utf8");
const llmsOutside = outsideCodeFences(llms).content;
const llmsMarkdownUrls = markdownLinks(llms).filter((href) =>
	new URL(href, origin).pathname.endsWith(".md"),
);

check(
	(llmsOutside.match(/^#\s+\S.*$/gm) ?? []).length === 1,
	"llms.txt: expected one H1",
);
check(
	/^# FlowBook\n\n>\s/m.test(llms),
	"llms.txt: missing title and blockquote summary",
);
check(llms.includes("## Start here"), "llms.txt: missing Start here section");
check(llms.includes("## Optional"), "llms.txt: missing Optional section");
check(
	llmsMarkdownUrls.length === new Set(llmsMarkdownUrls).size,
	"llms.txt: duplicate Markdown links found",
);
check(
	new Set(llmsMarkdownUrls).size === expectedMarkdownPaths.size,
	"llms.txt: Markdown link count does not match the public collection",
);
for (const markdownPath of expectedMarkdownPaths) {
	const markdownUrl = new URL(markdownPath, origin).toString();
	check(
		llmsMarkdownUrls.includes(markdownUrl),
		`llms.txt: missing ${markdownUrl}`,
	);
}
for (const href of markdownLinks(llms)) await checkLocalUrl(href, "/llms.txt");

const fullCanonicalPaths = [
	...llmsFull.matchAll(/- \*\*Canonical HTML:\*\* \[[^\]]+\]\(([^)]+)\)/g),
].map((match) => new URL(match[1]).pathname);
const expectedReadingPaths = canonicalPaths
	.filter(
		(pathname) =>
			pathname === "/introduction/" ||
			/^\/part-\d+\/\d{2}-[^/]+\/$/.test(pathname),
	)
	.sort((left, right) => {
		if (left === "/introduction/") return -1;
		if (right === "/introduction/") return 1;
		const leftChapter = Number(left.match(/\/(\d{2})-/)?.[1] ?? 0);
		const rightChapter = Number(right.match(/\/(\d{2})-/)?.[1] ?? 0);
		return leftChapter - rightChapter;
	});
check(
	JSON.stringify(fullCanonicalPaths) === JSON.stringify(expectedReadingPaths),
	"llms-full.txt: reading units are missing, duplicated, or out of order",
);
check(
	llmsFull.length > 100_000 && llmsFull.length < 5_000_000,
	"llms-full.txt: aggregate size is unexpectedly small or large",
);
const fullOutside = outsideCodeFences(llmsFull);
check(fullOutside.balanced, "llms-full.txt: unbalanced fenced code block");
check(
	!containsRawEsm(fullOutside.content) &&
		!/^\s*<\/?[A-Z][A-Za-z0-9]*(?:\s|\/?>)/m.test(fullOutside.content),
	"llms-full.txt: raw MDX syntax remains",
);

check(
	!sitemap.includes(".md</loc>"),
	"sitemap: Markdown alternate was included",
);
check(!sitemap.includes("llms.txt"), "sitemap: llms.txt was included");
check(
	!sitemap.includes("llms-full.txt"),
	"sitemap: llms-full.txt was included",
);

const robots = await readFile(path.join(distRoot, "robots.txt"), "utf8");
check(!/Disallow:\s*\/llms/i.test(robots), "robots.txt: LLM index is blocked");
check(
	!/Disallow:.*\.md/i.test(robots),
	"robots.txt: Markdown alternates are blocked",
);

const headers = await readFile(path.join(distRoot, "_headers"), "utf8");
const headerRules = parseHeaderRules(headers);
for (const route of ["/llms.txt", "/llms-full.txt"]) {
	const rule = headerRules.get(route);
	check(
		rule?.get("content-type")?.toLowerCase() === "text/markdown; charset=utf-8",
		`_headers: ${route} must use the Markdown MIME type`,
	);
	check(
		rule?.get("x-robots-tag")?.replaceAll(" ", "").toLowerCase() ===
			"noindex,follow",
		`_headers: ${route} must be noindex, follow`,
	);
}
check(
	headerRules.get("/*.md")?.get("content-type")?.toLowerCase() ===
		"text/markdown; charset=utf-8",
	"_headers: Markdown files must use text/markdown; charset=utf-8",
);
check(
	headerRules.get("/index.md")?.get("link")?.includes('rel="canonical"'),
	"_headers: root Markdown canonical Link header is missing",
);
check(
	headerRules.get("/*/index.md")?.get("link")?.includes(":splat/"),
	"_headers: nested Markdown canonical Link header is missing",
);
check(
	!headerRules.get("/*.md")?.has("x-robots-tag"),
	"_headers: equivalent per-page Markdown should use canonicalization, not noindex",
);

if (failures.length > 0) {
	console.error(
		`LLM content validation failed with ${failures.length} issue(s):`,
	);
	for (const failure of failures) console.error(`- ${failure}`);
	process.exit(1);
}

console.log(
	`LLM content validation passed for ${markdownFiles.length} Markdown pages, llms.txt, and ${(llmsFull.length / 1024).toFixed(1)} KiB of full-book context.`,
);
