import { readFile, readdir, stat } from "node:fs/promises";
import { dirname, resolve, relative, extname } from "node:path";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";
import * as cheerio from "cheerio";
import { sidebar } from "../src/sidebar.mjs";

// Use the YAML parser already owned by Astro, including with isolated installs.
const require = createRequire(import.meta.url);
const { load: loadYaml } = createRequire(require.resolve("astro/package.json"))(
	"js-yaml",
);
const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const contentRoot = resolve(root, "src/content/docs");
const errors = [];
const origin = "https://docs.flow-like.com";

function internalUrl(value, from) {
	try {
		const url = new URL(value, `${origin}/${from}`);
		return url.origin === origin ? url : undefined;
	} catch {
		errors.push(`${from}: invalid URL ${value}`);
	}
}

function decode(value, from) {
	try {
		return decodeURIComponent(value);
	} catch {
		errors.push(`${from}: invalid URL encoding ${value}`);
	}
}

function sourceSet(value = "") {
	const candidates = [];
	let rest = value;
	while ((rest = rest.replace(/^[\s,]+/, ""))) {
		const token = /^\S+/.exec(rest)[0];
		rest = rest.slice(token.length);
		let descriptor = "";
		if (!token.endsWith(",")) {
			const end = rest.indexOf(",");
			descriptor = (end < 0 ? rest : rest.slice(0, end)).trim();
			rest = end < 0 ? "" : rest.slice(end + 1);
		}
		const width = /^(\d+)w$/.exec(descriptor);
		candidates.push({
			src: token.replace(/,+$/, ""),
			width: width ? Number(width[1]) : undefined,
		});
	}
	return candidates;
}

async function files(directory) {
	const entries = await readdir(directory, { withFileTypes: true });
	return (
		await Promise.all(
			entries.map((entry) => {
				const path = resolve(directory, entry.name);
				return entry.isDirectory() ? files(path) : [path];
			}),
		)
	).flat();
}

const pages = new Map();
for (const path of await files(contentRoot)) {
	if (![".md", ".mdx"].includes(extname(path))) continue;
	const source = await readFile(path, "utf8");
	const slug = relative(contentRoot, path)
		.replace(/\.mdx?$/, "")
		.replace(/\/index$/, "");
	let frontmatter;
	const header =
		/^\uFEFF?---[ \t]*\r?\n([\s\S]*?)\r?\n(?:---|\.\.\.)[ \t]*(?:\r?\n|$)/.exec(
			source,
		);
	if (!header) errors.push(`${slug}: missing or unclosed YAML frontmatter`);
	else {
		try {
			frontmatter = loadYaml(header[1]);
			if (
				!frontmatter ||
				typeof frontmatter !== "object" ||
				Array.isArray(frontmatter)
			)
				errors.push(`${slug}: frontmatter must be a YAML mapping`);
		} catch (error) {
			errors.push(`${slug}: invalid YAML frontmatter: ${error.message}`);
		}
	}
	pages.set(slug, {
		path,
		source,
		hidden: frontmatter?.sidebar?.hidden === true,
	});
	let fence;
	for (const [index, line] of source.split("\n").entries()) {
		const match = /^\s*(`{3,}|~{3,})(.*)$/.exec(line);
		if (!match) continue;
		if (!fence) fence = { marker: match[1], line: index + 1 };
		else if (
			match[1][0] === fence.marker[0] &&
			match[1].length >= fence.marker.length &&
			!match[2].trim()
		)
			fence = undefined;
	}
	if (fence) errors.push(`${slug}:${fence.line}: unclosed code fence`);
}

const counts = new Map();
function add(slug) {
	if (!pages.has(slug)) errors.push(`Sidebar target does not exist: ${slug}`);
	counts.set(slug, (counts.get(slug) ?? 0) + 1);
}
function visit(items) {
	for (const item of items) {
		if (item.slug) add(item.slug);
		if (item.items) visit(item.items);
		if (item.autogenerate) {
			const prefix = item.autogenerate.directory;
			for (const [slug, page] of pages) {
				if ((slug === prefix || slug.startsWith(`${prefix}/`)) && !page.hidden)
					add(slug);
			}
		}
	}
}
visit(sidebar);
for (const [slug, page] of pages) {
	if (slug.startsWith("nodes/") || page.hidden) continue;
	if (counts.get(slug) !== 1)
		errors.push(
			`${slug}: expected one sidebar entry, found ${counts.get(slug) ?? 0}`,
		);
}

if (!process.argv.includes("--source-only")) {
	const dist = resolve(root, "dist");
	const all = await files(dist);
	const existing = new Set(all.map((path) => relative(dist, path)));
	const ids = new Map();
	const destinations = new Map();
	const imageSizes = new Map();
	const checkedVariants = new Set();
	const catalogHtml = await readFile(
		resolve(dist, "nodes/overview/index.html"),
		"utf8",
	);
	const catalogMarkdown = await readFile(
		resolve(dist, "nodes/overview/index.md"),
		"utf8",
	);
	const catalog = cheerio.load(catalogHtml, { scriptingEnabled: false });
	catalog("noscript a[href]").each((_, el) => {
		const href = catalog(el).attr("href");
		if (!catalogMarkdown.includes(`](${new URL(href, origin).href})`)) {
			errors.push(`nodes/overview/index.md: missing catalog link ${href}`);
		}
	});
	for (const path of [
		"topics/genai/prompt-templates/index.md",
		"topics/datascience/visualization/index.md",
	]) {
		if (
			/Loading chart(?: preview)?[.…]/.test(
				await readFile(resolve(dist, path), "utf8"),
			)
		) {
			errors.push(`${path}: chart hydration placeholder leaked into Markdown`);
		}
	}
	const imageSize = (name) => {
		if (!imageSizes.has(name))
			imageSizes.set(
				name,
				stat(resolve(dist, name)).then(({ size }) => size),
			);
		return imageSizes.get(name);
	};
	let htmlBytes = 0;
	let htmlPages = 0;
	for (const path of all.filter((path) => path.endsWith(".html"))) {
		const name = relative(dist, path);
		const source = await readFile(path, "utf8");
		htmlBytes += Buffer.byteLength(source);
		htmlPages++;
		const $ = cheerio.load(source);
		ids.set(
			name,
			new Set(
				$("[id], a[name]")
					.map((_, el) => [$(el).attr("id"), $(el).attr("name")])
					.get()
					.filter(Boolean),
			),
		);
		$("a[href]").each((_, el) => {
			const href = $(el).attr("href");
			const url = internalUrl(href, name);
			if (!url) return;
			destinations.set(url.href, destinations.get(url.href) ?? name);
		});
		const pageImages = new Set();
		for (const el of $("img, picture source[srcset]").toArray()) {
			const image = $(el);
			const candidates = sourceSet(image.attr("srcset"));
			const src = image.attr("src");
			const localCandidates = [];
			const article =
				image.closest(".sl-markdown-content").length &&
				!image.closest(".node-doc, .node-icon-frame").length;
			for (const candidate of [...candidates, ...(src ? [{ src }] : [])]) {
				const url = internalUrl(candidate.src, name);
				if (!url) continue;
				const target = decode(url.pathname, name)?.replace(/^\//, "");
				if (target === undefined) continue;
				if (!existing.has(target)) {
					errors.push(`${name}: missing image ${candidate.src}`);
					continue;
				}
				localCandidates.push({ ...candidate, target });
				if (
					article &&
					candidate.width <= 1600 &&
					!checkedVariants.has(target)
				) {
					checkedVariants.add(target);
					const bytes = await imageSize(target);
					if (bytes > 450_000)
						errors.push(
							`${name}: image ${target} (${candidate.width}w) is ${bytes} bytes (budget 450000)`,
						);
				}
			}
			if (article && el.tagName === "img") {
				const widths = localCandidates
					.filter(({ width }) => width)
					.sort((a, b) => a.width - b.width);
				const selected =
					widths.find(({ width }) => width >= 800) ??
					widths.at(-1) ??
					localCandidates.at(-1);
				if (selected) pageImages.add(selected.target);
			}
		}
		const pageImageBytes = (
			await Promise.all([...pageImages].map(imageSize))
		).reduce((total, bytes) => total + bytes, 0);
		if (pageImageBytes > 1_500_000)
			errors.push(
				`${name}: article images total ${pageImageBytes} bytes (budget 1500000 at 800w)`,
			);
		if (name !== "404.html" && !$("[data-pagefind-filter^='Section:']").length)
			errors.push(`${name}: missing search scope`);
		// Keep copyable Markdown examples intact after MDX compilation.
		$("pre").each((_, el) => {
			if (/^#{2,4} (?:Example output|Template:)/m.test($(el).text()))
				errors.push(`${name}: a heading was swallowed by a code block`);
		});
		const navLeaves = $("#starlight__sidebar a[href^='/nodes/']").length;
		if (navLeaves > 50)
			errors.push(
				`${name}: ${navLeaves} node links in global navigation (budget 50)`,
			);
	}
	for (const [href, from] of destinations) {
		const url = new URL(href);
		const path = decode(url.pathname, from)?.replace(/^\//, "");
		if (path === undefined) continue;
		const target = [
			path,
			`${path}index.html`,
			`${path.replace(/\/$/, "")}/index.html`,
			`${path}.html`,
		].find((name) => existing.has(name));
		if (!target) errors.push(`${from}: missing target ${url.pathname}`);
		else if (url.hash && ids.has(target)) {
			// Browsers accept named anchors, #top, and text fragments without an ID.
			const fragment = decode(url.hash.slice(1).split(":~:")[0], from);
			if (
				fragment &&
				fragment.toLowerCase() !== "top" &&
				!ids.get(target).has(fragment)
			)
				errors.push(`${from}: missing heading ${url.pathname}${url.hash}`);
		}
	}
	for (const [page, budget] of [
		["start/getting-started/index.html", 200_000],
		["nodes/overview/index.html", 2_000_000],
	]) {
		const bytes = (await stat(resolve(dist, page))).size;
		if (bytes > budget)
			errors.push(`${page}: ${bytes} bytes exceeds ${budget} byte HTML budget`);
	}
	console.log(
		`Checked ${htmlPages} built pages, ${destinations.size} internal destinations, ${(htmlBytes / 1024 / 1024).toFixed(1)} MiB HTML.`,
	);
}

if (errors.length) {
	console.error([...new Set(errors)].join("\n"));
	process.exitCode = 1;
} else {
	console.log(`Documentation checks passed for ${pages.size} source pages.`);
}
