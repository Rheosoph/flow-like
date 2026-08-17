/**
 * Build-time HTML → Markdown conversion for agent content negotiation.
 *
 * Mirror of apps/website/scripts/agent-markdown/html-to-markdown.mjs — keep both
 * copies in sync. The module is intentionally site agnostic; every site
 * specific decision (content selector, noise selectors, origin) is passed in
 * by the caller.
 */

import * as cheerio from "cheerio";

const BLOCK_CONTENT = "h1,h2,h3,h4,h5,h6,p,div,section,article,ul,ol,table,pre";

const VOID_OR_IGNORED = new Set([
	"area",
	"base",
	"col",
	"colgroup",
	"head",
	"map",
	"param",
	"source",
	"track",
	"wbr",
]);

export const DEFAULT_NOISE_SELECTORS = [
	"script",
	"style",
	"noscript",
	"template",
	"svg",
	"canvas",
	"iframe",
	"object",
	"embed",
	"form",
	"input:not([type='checkbox'])",
	"select",
	"textarea",
	"button",
	"link",
	"meta",
	"dialog",
	"[aria-hidden='true']",
	"[hidden]",
	"[data-pagefind-ignore]",
	".sr-only",
];

/** Only for values read straight out of the raw HTML, where no parser ran. */
function decodeMetaEntities(value) {
	return value
		.replace(/&#x([0-9a-f]+);/gi, (_, hex) =>
			String.fromCodePoint(Number.parseInt(hex, 16)),
		)
		.replace(/&#(\d+);/g, (_, dec) =>
			String.fromCodePoint(Number.parseInt(dec, 10)),
		)
		.replace(/&lt;/g, "<")
		.replace(/&gt;/g, ">")
		.replace(/&quot;/g, '"')
		.replace(/&nbsp;/g, " ")
		.replace(/&amp;/g, "&");
}

function matchTag(html, tag) {
	const token = new RegExp(`<${tag}(?=[\\s/>])|</${tag}\\s*>`, "gi");
	let start = -1;
	let depth = 0;

	for (const match of html.matchAll(token)) {
		const closing = match[0][1] === "/";
		if (start === -1) {
			if (closing) return null;
			start = match.index;
			depth = 1;
			continue;
		}

		depth += closing ? -1 : 1;
		if (depth === 0) return html.slice(start, match.index + match[0].length);
	}

	return start === -1 ? null : html.slice(start);
}

/**
 * Narrows a full HTML document to the region worth converting without paying
 * for a DOM of the whole page (built pages carry hundreds of KB of chrome).
 */
export function extractContentHtml(html, tags = ["main", "article", "body"]) {
	for (const tag of tags) {
		const region = matchTag(html, tag);
		if (region) return region;
	}
	return html;
}

/** Reads an attribute off a raw tag, quoted or minified (`id=content`). */
function attr(tag, name) {
	const match = new RegExp(
		`\\b${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)'|([^\\s>]+))`,
		"i",
	).exec(tag);
	if (!match) return "";
	return decodeMetaEntities(match[1] ?? match[2] ?? match[3] ?? "").trim();
}

function tagWithAttribute(html, tag, name, value) {
	const tags = html.matchAll(new RegExp(`<${tag}\\b[^>]*>`, "gi"));
	for (const [candidate] of tags) {
		if (attr(candidate, name).toLowerCase() === value) return candidate;
	}
	return "";
}

export function extractPageMeta(html) {
	const headEnd = html.indexOf("</head>");
	const head = headEnd === -1 ? html : html.slice(0, headEnd);
	const title = /<title[^>]*>([\s\S]*?)<\/title>/i.exec(head);
	const description =
		tagWithAttribute(head, "meta", "name", "description") ||
		tagWithAttribute(head, "meta", "property", "og:description");
	const htmlTag = /<html\b[^>]*>/i.exec(html);

	return {
		title: title ? decodeMetaEntities(title[1]).trim() : "",
		description: description ? attr(description, "content") : "",
		lang: htmlTag ? attr(htmlTag[0], "lang") : "",
		canonical: attr(tagWithAttribute(head, "link", "rel", "canonical"), "href"),
	};
}

function absoluteUrl(value, baseUrl) {
	if (!value) return "";
	const trimmed = value.trim();
	if (!trimmed || trimmed.startsWith("#") || trimmed.startsWith("data:")) {
		return trimmed;
	}
	if (!baseUrl) return trimmed;
	try {
		return new URL(trimmed, baseUrl).toString();
	} catch {
		return trimmed;
	}
}

function collapseWhitespace(value) {
	return value.replace(/[\t\n\r\f\v ]+/g, " ");
}

function escapeInline(value) {
	return value.replace(/([\\`*[\]])/g, "\\$1");
}

function indentLines(value, indent, firstLinePrefix = indent) {
	const lines = value.split("\n");
	return lines
		.map((line, index) => {
			if (index === 0) return `${firstLinePrefix}${line}`;
			return line.length ? `${indent}${line}` : line;
		})
		.join("\n");
}

function block(content) {
	const trimmed = content.trim();
	return trimmed ? `\n\n${trimmed}\n\n` : "";
}

function textOf($, node) {
	return $(node).text();
}

function codeLanguage($, pre) {
	const explicit = pre.attribs?.["data-language"];
	if (explicit && explicit !== "plaintext") return explicit;

	const classNames = [
		pre.attribs?.class ?? "",
		$(pre).find("code").first().attr("class") ?? "",
	].join(" ");
	const match = /(?:language|lang)-([\w+#-]+)/.exec(classNames);
	return match ? match[1] : "";
}

function codeContent($, pre) {
	const lineNodes = $(pre).find(".ec-line, .line");
	if (lineNodes.length) {
		return lineNodes
			.map((_, line) => $(line).text())
			.get()
			.join("\n");
	}
	return $(pre).text();
}

function fence(code, language) {
	let ticks = 3;
	for (const [run] of code.matchAll(/`{3,}/g)) {
		ticks = Math.max(ticks, run.length + 1);
	}
	const marker = "`".repeat(ticks);
	return `${marker}${language}\n${code.replace(/\n+$/, "")}\n${marker}`;
}

function inlineCode(value) {
	let ticks = 1;
	for (const [run] of value.matchAll(/`+/g)) {
		ticks = Math.max(ticks, run.length + 1);
	}
	const marker = "`".repeat(ticks);
	const padding = value.startsWith("`") || value.endsWith("`") ? " " : "";
	return `${marker}${padding}${value}${padding}${marker}`;
}

function renderTable($, node, ctx) {
	const table = $(node)
		.find("tr")
		.get()
		.map((tr) =>
			$(tr)
				.children("th,td")
				.get()
				.map((cell) =>
					collapseWhitespace(renderChildren($, cell, { ...ctx, inTable: true }))
						.replace(/\|/g, "\\|")
						.trim(),
				),
		)
		.filter((row) => row.length);

	if (!table.length) return "";

	const width = Math.max(...table.map((row) => row.length));
	const normalize = (row) =>
		`| ${Array.from({ length: width }, (_, i) => row[i] ?? "").join(" | ")} |`;

	const hasHead = $(node).find("th").length > 0;
	const header = hasHead ? table[0] : Array.from({ length: width }, () => "");
	const body = hasHead ? table.slice(1) : table;

	return [
		normalize(header),
		`| ${Array.from({ length: width }, () => "---").join(" | ")} |`,
		...body.map(normalize),
	].join("\n");
}

function renderList($, node, ctx) {
	const ordered = node.name === "ol";
	const start = Number.parseInt(node.attribs?.start ?? "1", 10) || 1;
	const items = $(node).children("li").get();

	const rendered = items.map((item, index) => {
		const marker = ordered ? `${start + index}. ` : "- ";
		const indent = " ".repeat(marker.length);
		const checkbox = $(item).children("input[type='checkbox']").first();
		const task = checkbox.length
			? checkbox.attr("checked") !== undefined
				? "[x] "
				: "[ ] "
			: "";
		const content = renderChildren($, item, {
			...ctx,
			listDepth: (ctx.listDepth ?? 0) + 1,
		})
			.replace(/\n{3,}/g, "\n\n")
			.trim();
		return indentLines(`${task}${content}`, indent, marker);
	});

	return rendered.filter(Boolean).join("\n");
}

function renderChildren($, node, ctx) {
	let out = "";
	let previousWasElement = false;

	for (const child of node.children ?? []) {
		const rendered = renderNode($, child, ctx);
		if (!rendered) continue;

		const isElement = child.type === "tag";
		const atLineStart = out.length === 0 || out.endsWith("\n");
		// Minified markup glues sibling elements together ("<i>JS</i><i>RUST</i>")
		// where CSS did the spacing; keep them apart so the markdown stays legible.
		const needsSeparator =
			isElement &&
			previousWasElement &&
			!atLineStart &&
			!/\s$/.test(out) &&
			!/^\s/.test(rendered);

		if (atLineStart) {
			// Source formatting must not indent a line — four spaces would read as
			// an indented code block.
			const trimmed = rendered.replace(/^[ \t]+/, "");
			if (!trimmed) continue;
			out += trimmed;
		} else {
			out += needsSeparator ? ` ${rendered}` : rendered;
		}

		previousWasElement = isElement;
	}

	return out;
}

function renderNode($, node, ctx) {
	if (node.type === "text") {
		const collapsed = collapseWhitespace(node.data ?? "");
		if (!collapsed.trim()) return collapsed.length ? " " : "";
		return ctx.raw ? collapsed : escapeInline(collapsed);
	}

	if (node.type !== "tag" && node.type !== "script" && node.type !== "style") {
		return "";
	}

	const name = node.name?.toLowerCase();
	if (!name || VOID_OR_IGNORED.has(name)) return "";

	switch (name) {
		case "br":
			return ctx.inTable ? " " : "  \n";
		case "hr":
			return block("---");
		case "h1":
		case "h2":
		case "h3":
		case "h4":
		case "h5":
		case "h6": {
			const text = collapseWhitespace(
				renderChildren($, node, { ...ctx, raw: true }),
			).trim();
			if (!text) return "";
			const level = Number.parseInt(name.slice(1), 10);
			return block(`${"#".repeat(level)} ${text}`);
		}
		case "pre": {
			const code = codeContent($, node);
			if (!code.trim()) return "";
			return block(fence(code, codeLanguage($, node)));
		}
		case "code":
		case "kbd":
		case "samp": {
			const value = collapseWhitespace(textOf($, node)).trim();
			return value ? inlineCode(value) : "";
		}
		case "blockquote": {
			const content = renderChildren($, node, ctx).trim();
			if (!content) return "";
			const quoted = content
				.split("\n")
				.map((line) => (line ? `> ${line}` : ">"))
				.join("\n");
			return block(quoted);
		}
		case "ul":
		case "ol": {
			const list = renderList($, node, ctx);
			if (!list) return "";
			return ctx.listDepth ? `\n${list}\n` : block(list);
		}
		case "li":
			return renderChildren($, node, ctx);
		case "table": {
			const table = renderTable($, node, ctx);
			return table ? block(table) : "";
		}
		case "thead":
		case "tbody":
		case "tfoot":
		case "tr":
		case "th":
		case "td":
			return renderChildren($, node, ctx);
		case "img": {
			const src = absoluteUrl(node.attribs?.src ?? "", ctx.baseUrl);
			if (!src) return "";
			const alt = collapseWhitespace(node.attribs?.alt ?? "").trim();
			return `![${escapeInline(alt)}](${src})`;
		}
		case "a": {
			// Cards and CTAs wrap whole blocks in an anchor; a link label has to
			// stay on one line, so headings and paragraph breaks are flattened.
			const label = collapseWhitespace(renderChildren($, node, ctx))
				.replace(/\\?#{1,6} /g, "")
				.trim();
			if (!label) return "";
			const href = absoluteUrl(node.attribs?.href ?? "", ctx.baseUrl);
			const link = href ? `[${label}](${href.replace(/\)/g, "%29")})` : label;
			// A card link swallowed a whole block of text; keeping it inline would
			// merge a grid of cards into one unreadable paragraph.
			return $(node).find(BLOCK_CONTENT).length ? block(link) : link;
		}
		case "strong":
		case "b": {
			const value = renderChildren($, node, ctx).trim();
			return value ? `**${value}**` : "";
		}
		case "em":
		case "i":
		case "cite": {
			const value = renderChildren($, node, ctx).trim();
			return value ? `*${value}*` : "";
		}
		case "del":
		case "s": {
			const value = renderChildren($, node, ctx).trim();
			return value ? `~~${value}~~` : "";
		}
		case "summary": {
			const value = collapseWhitespace(renderChildren($, node, ctx)).trim();
			return value ? block(`**${value}**`) : "";
		}
		case "dt": {
			const value = collapseWhitespace(renderChildren($, node, ctx)).trim();
			return value ? block(`**${value}**`) : "";
		}
		case "dd": {
			const value = renderChildren($, node, ctx).trim();
			return value ? block(indentLines(value, "  ")) : "";
		}
		case "p":
		case "div":
		case "section":
		case "article":
		case "aside":
		case "header":
		case "footer":
		case "main":
		case "nav":
		case "figure":
		case "figcaption":
		case "details":
		case "dl":
		case "address":
		case "fieldset":
		case "blockquote-footer":
			return block(renderChildren($, node, ctx));
		default:
			return renderChildren($, node, ctx);
	}
}

function tidy(markdown) {
	return markdown
		.replace(/ /g, " ")
		.replace(/[ \t]+\n/g, "\n")
		.replace(/\n{3,}/g, "\n\n")
		.replace(/^\s+|\s+$/g, "");
}

/**
 * Converts an HTML fragment to Markdown.
 *
 * @param {string} html fragment to convert
 * @param {{ baseUrl?: string, noiseSelectors?: string[], contentSelector?: string }} options
 */
export function htmlToMarkdown(html, options = {}) {
	const {
		baseUrl = "",
		noiseSelectors = DEFAULT_NOISE_SELECTORS,
		contentSelector = "",
	} = options;

	const $ = cheerio.load(html, null, false);
	if (noiseSelectors.length) $(noiseSelectors.join(",")).remove();

	const root =
		contentSelector && $(contentSelector).length
			? $(contentSelector).first()[0]
			: $.root()[0];

	return tidy(renderChildren($, root, { baseUrl }));
}

export function frontmatter(fields) {
	const lines = Object.entries(fields)
		.filter(
			([, value]) => value !== undefined && value !== null && value !== "",
		)
		.map(([key, value]) => {
			const normalized = String(value).replace(/\s+/g, " ").trim();
			return `${key}: ${JSON.stringify(normalized)}`;
		});
	return lines.length ? `---\n${lines.join("\n")}\n---\n\n` : "";
}

/**
 * Rough token estimate, mirroring the heuristic Cloudflare reports through
 * `x-markdown-tokens` (~4 characters per token).
 */
export function estimateTokens(text) {
	return Math.max(1, Math.ceil(text.length / 4));
}
