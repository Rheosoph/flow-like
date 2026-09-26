type HtmlElement = {
	readonly tag: string;
	readonly attributes: ReadonlyArray<readonly [string, string]>;
	readonly children: HtmlNode[];
};
type HtmlNode = HtmlElement | { readonly text: string };

const TOKEN =
	/<!--[\s\S]*?-->|<(\/?)([a-zA-Z][\w:.-]*)((?:\s+[^\s"'<>/=]+(?:\s*=\s*(?:"[^"]*"|'[^']*'|[^\s"'<>=`]+))?)*)\s*(\/?)>|[^<]+|</g;
const ATTRIBUTE =
	/([^\s"'<>/=]+)(?:\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s"'<>=`]+)))?/g;

const VOID_ELEMENTS = new Set([
	"area",
	"base",
	"br",
	"col",
	"embed",
	"hr",
	"img",
	"input",
	"link",
	"meta",
	"source",
	"track",
	"wbr",
]);

/** React useId, Radix and dnd-kit stamp render-position counters, not content. */
const GENERATED_ID_PATTERNS: ReadonlyArray<readonly [RegExp, string]> = [
	[/DndDescribedBy-\d+/g, "DndDescribedBy-N"],
	[/DndLiveRegion-\d+/g, "DndLiveRegion-N"],
	[/radix-[\w-]+/g, "radix-N"],
	[/«[^»]*»/g, "«N»"],
	[/:r[0-9a-z]+:/g, ":rN:"],
	[/_r_[0-9a-z]+_/g, "_rN_"],
];

const INVISIBLE = /[\u00a0\u00ad\u200b-\u200f\u2028\u2029\u2060\ufeff]/g;

const REACT_BLOCKED_URL =
	"javascript:throw new Error('React has blocked a javascript: URL as a security precaution.')";

const DANGEROUS_ELEMENTS = new Set([
	"base",
	"embed",
	"form",
	"frame",
	"frameset",
	"iframe",
	"link",
	"meta",
	"object",
	"script",
	"style",
]);

const URL_ATTRIBUTES = new Set([
	"action",
	"background",
	"data",
	"formaction",
	"href",
	"poster",
	"src",
	"srcdoc",
	"xlink:href",
]);

const DANGEROUS_URL = /^(?:javascript|vbscript):|^data:text\/html/i;

const ENTITIES: Record<string, string> = {
	amp: "&",
	apos: "'",
	gt: ">",
	lt: "<",
	nbsp: "\u00a0",
	quot: '"',
};

export function decodeEntities(value: string): string {
	return value.replace(
		/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi,
		(match, entity: string) => {
			if (entity[0] !== "#") return ENTITIES[entity.toLowerCase()] ?? match;
			const code =
				entity[1] === "x" || entity[1] === "X"
					? Number.parseInt(entity.slice(2), 16)
					: Number.parseInt(entity.slice(1), 10);
			return Number.isFinite(code) ? String.fromCodePoint(code) : match;
		},
	);
}

function parseAttributes(source: string): Array<readonly [string, string]> {
	const attributes: Array<readonly [string, string]> = [];
	for (const match of source.matchAll(ATTRIBUTE)) {
		attributes.push([
			match[1].toLowerCase(),
			match[2] ?? match[3] ?? match[4] ?? "",
		]);
	}
	return attributes;
}

/** Tolerant tokenizer for React/happy-dom serialized markup (quoted attributes). */
export function parseHtml(html: string): HtmlNode[] {
	const root: HtmlElement = { tag: "#root", attributes: [], children: [] };
	const stack: HtmlElement[] = [root];
	for (const match of html.matchAll(TOKEN)) {
		const [token, closing, rawTag, rawAttributes, selfClosing] = match;
		const parent = stack[stack.length - 1];
		if (token.startsWith("<!--")) continue;
		if (!rawTag) {
			parent.children.push({ text: token });
			continue;
		}
		const tag = rawTag.toLowerCase();
		if (closing) {
			const index = stack.findLastIndex((element) => element.tag === tag);
			if (index > 0) stack.length = index;
			continue;
		}
		const element: HtmlElement = {
			tag,
			attributes: parseAttributes(rawAttributes ?? ""),
			children: [],
		};
		parent.children.push(element);
		if (!selfClosing && !VOID_ELEMENTS.has(tag)) stack.push(element);
	}
	return root.children;
}

const isElement = (node: HtmlNode): node is HtmlElement => "tag" in node;

function quoteText(text: string): string {
	return JSON.stringify(text).replace(
		INVISIBLE,
		(char) => `\\u${char.charCodeAt(0).toString(16).padStart(4, "0")}`,
	);
}

export type FormatOptions = {
	/**
	 * Attributes whose values are random per mount (Plate node ids). Each distinct
	 * value becomes a stable ordinal so structure and reuse stay visible.
	 */
	readonly volatileAttributes?: ReadonlyArray<string>;
};

function normalizeAttributeValue(name: string, value: string): string {
	let normalized = value;
	for (const [pattern, replacement] of GENERATED_ID_PATTERNS)
		normalized = normalized.replace(pattern, replacement);
	if (name === "class")
		normalized = normalized.split(/\s+/).filter(Boolean).sort().join(" ");
	return normalized;
}

/**
 * Pretty-prints markup deterministically: one node per line, attributes and
 * class tokens sorted, generated ids replaced, SVG bodies (icon paths) elided.
 * Text nodes are JSON-quoted so whitespace and invisible characters show.
 */
export function formatHtml(html: string, options: FormatOptions = {}): string {
	const volatile = new Set(options.volatileAttributes ?? []);
	const ordinals = new Map<string, string>();
	const ordinal = (value: string) => {
		let mapped = ordinals.get(value);
		if (!mapped) {
			mapped = `<volatile-${ordinals.size + 1}>`;
			ordinals.set(value, mapped);
		}
		return mapped;
	};

	const renderAttributes = (element: HtmlElement) =>
		[...element.attributes]
			.map(([name, value]) => {
				const normalized = volatile.has(name)
					? ordinal(value)
					: normalizeAttributeValue(name, value);
				return `${name}="${normalized}"`;
			})
			.sort()
			.map((attribute) => ` ${attribute}`)
			.join("");

	const out: string[] = [];
	const print = (node: HtmlNode, depth: number) => {
		const indent = "  ".repeat(depth);
		if (!isElement(node)) {
			out.push(`${indent}${quoteText(node.text)}`);
			return;
		}
		const attributes = renderAttributes(node);
		if (node.tag === "svg" || VOID_ELEMENTS.has(node.tag)) {
			out.push(`${indent}<${node.tag}${attributes} />`);
			return;
		}
		const [only] = node.children;
		if (node.children.length === 0) {
			out.push(`${indent}<${node.tag}${attributes}></${node.tag}>`);
		} else if (node.children.length === 1 && !isElement(only)) {
			out.push(
				`${indent}<${node.tag}${attributes}>${quoteText(only.text)}</${node.tag}>`,
			);
		} else {
			out.push(`${indent}<${node.tag}${attributes}>`);
			for (const child of node.children) print(child, depth + 1);
			out.push(`${indent}</${node.tag}>`);
		}
	};
	for (const node of parseHtml(html)) print(node, 0);
	return out.join("\n");
}

/** Decoded text content, as a browser would show it. */
export function textOf(html: string): string {
	const collect = (nodes: HtmlNode[]): string =>
		nodes
			.map((node) => (isElement(node) ? collect(node.children) : node.text))
			.join("");
	return decodeEntities(collect(parseHtml(html)));
}

export function countElements(
	html: string,
	predicate: (tag: string, attributes: Map<string, string>) => boolean,
): number {
	let count = 0;
	const visit = (nodes: HtmlNode[]) => {
		for (const node of nodes) {
			if (!isElement(node)) continue;
			if (predicate(node.tag, new Map(node.attributes))) count++;
			visit(node.children);
		}
	};
	visit(parseHtml(html));
	return count;
}

/** React 19 server rendering hoists `<link rel="preload" as="image">` for every `<img>`. */
const isImagePreload = (element: HtmlElement) => {
	const attributes = new Map(element.attributes);
	return (
		element.tag === "link" &&
		attributes.get("rel") === "preload" &&
		attributes.get("as") === "image"
	);
};

/**
 * Anything that could execute: dangerous elements, `on*` attributes, script
 * URLs in URL-bearing attributes and style expressions. React's own
 * "blocked a javascript: URL" replacement is inert and not reported.
 */
export function findUnsafeMarkup(html: string): string[] {
	const hits = new Set<string>();
	const visit = (nodes: HtmlNode[]) => {
		for (const node of nodes) {
			if (!isElement(node)) continue;
			const { tag } = node;
			if (DANGEROUS_ELEMENTS.has(tag) && !isImagePreload(node))
				hits.add(`element:${tag}`);
			for (const [name, raw] of node.attributes) {
				const value = decodeEntities(raw);
				if (name.startsWith("on")) hits.add(`attribute:${tag}[${name}]`);
				if (URL_ATTRIBUTES.has(name) && value !== REACT_BLOCKED_URL) {
					const compact = [...value]
						.filter((char) => char.charCodeAt(0) > 0x20)
						.join("");
					if (DANGEROUS_URL.test(compact)) hits.add(`url:${tag}[${name}]`);
				}
				if (name === "style" && /javascript:|expression\(/i.test(value))
					hits.add(`style:${tag}`);
			}
			visit(node.children);
		}
	};
	visit(parseHtml(html));
	return [...hits].sort();
}

export type Rendered =
	| { readonly html: string; readonly error?: undefined }
	| { readonly html?: undefined; readonly error: string };

export const describeError = (error: unknown) =>
	String(error instanceof Error ? `${error.name}: ${error.message}` : error)
		.split("\n")[0]
		.slice(0, 160);

export function captureRender(render: () => string): Rendered {
	try {
		return { html: render() };
	} catch (error) {
		return { error: describeError(error) };
	}
}

/** Formatted markup, or the error the render threw, so crashes are pinned too. */
export function formatRendered(
	rendered: Rendered,
	options?: FormatOptions,
): string {
	return rendered.error === undefined
		? formatHtml(rendered.html, options)
		: `THROWS ${rendered.error}`;
}

export const escapeInvisible = (text: string) => quoteText(text).slice(1, -1);

/**
 * A compact structural summary for renders whose full markup is already pinned
 * elsewhere: visible text, element counts keyed by tag and Slate type, and the
 * distinct inline styles (indent, alignment, colours, sizes).
 */
export function fingerprint(rendered: Rendered) {
	if (rendered.error !== undefined) return { throws: rendered.error };
	const elements = new Map<string, number>();
	const styles = new Set<string>();
	const visit = (nodes: HtmlNode[]) => {
		for (const node of nodes) {
			if (!isElement(node)) continue;
			const attributes = new Map(node.attributes);
			const type = attributes.get("data-slate-type");
			const key = type ? `${node.tag}[${type}]` : node.tag;
			elements.set(key, (elements.get(key) ?? 0) + 1);
			const style = attributes.get("style");
			if (style) styles.add(style);
			visit(node.children);
		}
	};
	visit(parseHtml(rendered.html));
	return {
		text: escapeInvisible(textOf(rendered.html)),
		elements: Object.fromEntries(
			[...elements].sort(([a], [b]) => (a < b ? -1 : 1)),
		),
		styles: [...styles].sort(),
	};
}

type PlateLikeNode = { [key: string]: unknown };

/**
 * Replaces random element ids (NodeIdPlugin) with a placeholder. Ids listed in
 * `keep` were part of the input and must survive verbatim.
 */
export function normalizeNodeIds<T>(value: T, keep?: ReadonlySet<string>): T {
	const visit = (node: unknown): unknown => {
		if (Array.isArray(node)) return node.map(visit);
		if (!node || typeof node !== "object") return node;
		const record = node as PlateLikeNode;
		const isElement = Array.isArray(record.children);
		const copy: PlateLikeNode = {};
		for (const [key, child] of Object.entries(record)) {
			copy[key] =
				isElement &&
				key === "id" &&
				typeof child === "string" &&
				!keep?.has(child)
					? "<node-id>"
					: visit(child);
		}
		return copy;
	};
	return visit(value) as T;
}

export function collectNodeIds(value: unknown): Set<string> {
	const ids = new Set<string>();
	const visit = (node: unknown) => {
		if (Array.isArray(node)) return node.forEach(visit);
		if (!node || typeof node !== "object") return;
		const record = node as PlateLikeNode;
		if (typeof record.id === "string") ids.add(record.id);
		if (Array.isArray(record.children)) record.children.forEach(visit);
	};
	visit(value);
	return ids;
}
