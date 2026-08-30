import { type WidgetContract, canonicalizeContract } from "./contract-types";
import { type TagSpan, findTag, insertAtHeadStart } from "./html";

export type AssetResolver = (relPath: string) => Uint8Array | null;

export interface InlineResult {
	html: string;
	/** Local asset paths that were inlined into the document */
	inlined: string[];
	/** Bundle-relative shared chunk paths (`shared/...`) the document references */
	external: string[];
}

type RefKind =
	| { kind: "url" }
	| { kind: "shared"; file: string }
	| { kind: "local"; path: string };

const SHARED_REF = /^(?:\.\/|\/)?(?:\.\.\/)*shared\/(.+)$/;

function classifyRef(ref: string): RefKind {
	if (/^[a-z][a-z0-9+.-]*:/i.test(ref) || ref.startsWith("//")) {
		return { kind: "url" };
	}
	const shared = SHARED_REF.exec(ref);
	if (shared?.[1]) {
		return { kind: "shared", file: shared[1] };
	}
	return { kind: "local", path: ref.replace(/^(?:\.\/|\/)+/, "") };
}

function attrValue(tag: string, name: string): string | null {
	const match = new RegExp(
		`\\b${name}\\s*=\\s*(?:"([^"]*)"|'([^']*)'|([^\\s>]+))`,
		"i",
	).exec(tag);
	if (!match) return null;
	return match[1] ?? match[2] ?? match[3] ?? null;
}

function replaceAttrValue(tag: string, name: string, value: string): string {
	return tag.replace(
		new RegExp(`(\\b${name}\\s*=\\s*)(?:"[^"]*"|'[^']*'|[^\\s>]+)`, "i"),
		`$1"${value}"`,
	);
}

function escapeScriptContent(js: string): string {
	return js.replace(/<\/script/gi, "<\\/script");
}

function escapeStyleContent(css: string): string {
	return css.replace(/<\/style/gi, "<\\/style");
}

const DECODER = new TextDecoder();

const WHITESPACE = /\s/;

interface ScriptElement extends TagSpan {
	attrs: string;
	body: string;
}

/** `String.replace` over hand-scanned spans; replacements are never rescanned. */
function rewriteSpans<T extends TagSpan>(
	html: string,
	find: (from: number) => T | null,
	replace: (span: T) => string,
): string {
	let out = "";
	let cursor = 0;
	for (;;) {
		const span = find(cursor);
		if (span === null) break;
		out += html.slice(cursor, span.start) + replace(span);
		cursor = span.end;
	}
	return out + html.slice(cursor);
}

function findScriptClose(
	html: string,
	from: number,
): { start: number; end: number } | null {
	let cursor = from;
	for (;;) {
		const start = html.indexOf("<", cursor);
		if (start === -1) return null;
		if (html.slice(start + 1, start + 8).toLowerCase() !== "/script") {
			cursor = start + 1;
			continue;
		}
		const tail = start + 8;
		const next = html[tail];
		if (next === ">") return { start, end: tail + 1 };
		if (next !== undefined && WHITESPACE.test(next)) {
			const gt = html.indexOf(">", tail + 1);
			return gt === -1 ? null : { start, end: gt + 1 };
		}
		cursor = start + 1;
	}
}

/** Linear-time scan for `<script …>…</script …>`; a regex here is quadratic. */
function findScriptElement(html: string, from: number): ScriptElement | null {
	const open = findTag(html, "script", from);
	if (open === null) return null;
	const close = findScriptClose(html, open.end);
	if (close === null) return null;
	return {
		start: open.start,
		end: close.end,
		attrs: html.slice(open.start + 7, open.end - 1),
		body: html.slice(open.end, close.start),
	};
}

/**
 * Inline local `<script src>` and stylesheet `<link>` references into the
 * document; rewrite references into `shared/` to bundle-relative
 * `../../shared/<file>` (the widget document lives at
 * `widgets/{id}/index.html`).
 */
export function inlineHtml(
	html: string,
	resolveAsset: AssetResolver,
): InlineResult {
	const inlined: string[] = [];
	const external = new Set<string>();

	const replaceScript = (element: ScriptElement): string => {
		const { attrs, body } = element;
		const full = html.slice(element.start, element.end);
		const src = attrValue(`<script${attrs}>`, "src");
		if (src === null) return full;
		const ref = classifyRef(src);
		if (ref.kind === "url") return full;
		if (ref.kind === "shared") {
			external.add(`shared/${ref.file}`);
			const openTag = replaceAttrValue(
				`<script${attrs}>`,
				"src",
				`../../shared/${ref.file}`,
			);
			return `${openTag}${body}</script>`;
		}
		const asset = resolveAsset(ref.path);
		if (asset === null) {
			throw new Error(
				`Cannot inline script '${src}': file not found next to the built document`,
			);
		}
		inlined.push(ref.path);
		const type = attrValue(`<script${attrs}>`, "type");
		const typeAttr = type ? ` type="${type}"` : "";
		return `<script${typeAttr}>${escapeScriptContent(DECODER.decode(asset))}</script>`;
	};

	const replaceLink = (full: string): string => {
		const rel = attrValue(full, "rel")?.toLowerCase();
		const href = attrValue(full, "href");
		if (!href || (rel !== "stylesheet" && rel !== "modulepreload")) {
			return full;
		}
		const ref = classifyRef(href);
		if (ref.kind === "url") return full;
		if (ref.kind === "shared") {
			external.add(`shared/${ref.file}`);
			return replaceAttrValue(full, "href", `../../shared/${ref.file}`);
		}
		if (rel === "modulepreload") {
			return "";
		}
		const asset = resolveAsset(ref.path);
		if (asset === null) {
			throw new Error(
				`Cannot inline stylesheet '${href}': file not found next to the built document`,
			);
		}
		inlined.push(ref.path);
		return `<style>${escapeStyleContent(DECODER.decode(asset))}</style>`;
	};

	const withScripts = rewriteSpans(
		html,
		(from) => findScriptElement(html, from),
		replaceScript,
	);
	const out = rewriteSpans(
		withScripts,
		(from) => findTag(withScripts, "link", from),
		(tag) => replaceLink(withScripts.slice(tag.start, tag.end)),
	);

	return { html: out, inlined, external: [...external].sort() };
}

export function contractScriptContent(contract: WidgetContract): string {
	const json = JSON.stringify(canonicalizeContract(contract)).replace(
		/</g,
		"\\u003c",
	);
	return `globalThis.__FLW_CONTRACT__ = ${json};`;
}

const CONTRACT_OPEN = "<script>";
const CONTRACT_CLOSE = "</script>";
const CONTRACT_MARKER = "globalThis.__FLW_CONTRACT__";

/** Linear-time removal of a previously injected contract script. */
function stripContractScript(html: string): string {
	const leadingWhitespace = /\s*/y;
	let from = 0;
	for (;;) {
		const open = html.indexOf(CONTRACT_OPEN, from);
		if (open === -1) return html;
		leadingWhitespace.lastIndex = open + CONTRACT_OPEN.length;
		leadingWhitespace.exec(html);
		const marker = leadingWhitespace.lastIndex;
		if (html.startsWith(CONTRACT_MARKER, marker)) {
			const close = html.indexOf(
				CONTRACT_CLOSE,
				marker + CONTRACT_MARKER.length,
			);
			if (close === -1) return html;
			return html.slice(0, open) + html.slice(close + CONTRACT_CLOSE.length);
		}
		from = open + 1;
	}
}

/**
 * Add `globalThis.__FLW_CONTRACT__` as the first script in `<head>`,
 * replacing a previously injected one (dev builds inject it via the Vite
 * plugin; pack is authoritative).
 */
export function injectContractScript(
	html: string,
	contract: WidgetContract,
): string {
	const stripped = stripContractScript(html);
	return insertAtHeadStart(
		stripped,
		`<script>${contractScriptContent(contract)}</script>`,
	);
}
