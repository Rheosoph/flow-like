import { type WidgetContract, canonicalizeContract } from "./contract-types";
import { insertAtHeadStart } from "./html";

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

	let out = html.replace(
		/<script\b([^>]*)>([\s\S]*?)<\/script\s*>/gi,
		(full, attrs: string, body: string) => {
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
		},
	);

	out = out.replace(/<link\b[^>]*\/?>/gi, (full) => {
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
	});

	return { html: out, inlined, external: [...external].sort() };
}

export function contractScriptContent(contract: WidgetContract): string {
	const json = JSON.stringify(canonicalizeContract(contract)).replace(
		/</g,
		"\\u003c",
	);
	return `globalThis.__FLW_CONTRACT__ = ${json};`;
}

const EXISTING_CONTRACT_SCRIPT =
	/<script>\s*globalThis\.__FLW_CONTRACT__[\s\S]*?<\/script>/;

/**
 * Add `globalThis.__FLW_CONTRACT__` as the first script in `<head>`,
 * replacing a previously injected one (dev builds inject it via the Vite
 * plugin; pack is authoritative).
 */
export function injectContractScript(
	html: string,
	contract: WidgetContract,
): string {
	const stripped = html.replace(EXISTING_CONTRACT_SCRIPT, "");
	return insertAtHeadStart(
		stripped,
		`<script>${contractScriptContent(contract)}</script>`,
	);
}
