import * as vscode from "vscode";

export interface ParamDoc {
	readonly name: string;
	readonly type: string;
	readonly optional: boolean;
	readonly doc?: string;
}

export interface ReturnDoc {
	readonly name?: string;
	readonly type: string;
	readonly doc?: string;
}

export interface NodeSignature {
	readonly name: string;
	readonly params: ParamDoc[];
	readonly returns: ReturnDoc[];
	readonly returnText: string;
	readonly description?: string;
	readonly impure: boolean;
	readonly source: vscode.Uri;
	readonly nameRange: vscode.Range;
}

export interface NodeSchemas {
	readonly inputs?: Readonly<Record<string, string>>;
	readonly outputs?: Readonly<Record<string, string>>;
}

const DECLARE_RE =
	/\/\*\*([\s\S]*?)\*\/\s*declare\s+function\s+([A-Za-z_$][\w$]*)\s*\(([\s\S]*?)\)\s*:\s*([^;]+);/g;
const DECLARE_NO_DOC_RE =
	/declare\s+function\s+([A-Za-z_$][\w$]*)\s*\(([\s\S]*?)\)\s*:\s*([^;]+);/g;

/**
 * Registry of all node signatures discovered from `.flow.d` files in the workspace.
 * Functions declared in a `.flow.d` are the catalog nodes callable from `.flow` files.
 */
export class SignatureRegistry {
	private readonly byName = new Map<string, NodeSignature>();
	private readonly schemas = new Map<string, NodeSchemas>();

	get size(): number {
		return this.byName.size;
	}

	all(): IterableIterator<NodeSignature> {
		return this.byName.values();
	}

	get(name: string): NodeSignature | undefined {
		return this.byName.get(name);
	}

	has(name: string): boolean {
		return this.byName.has(name);
	}

	/** Per-pin JSON Schema strings for a node, loaded from a `.flow.schemas.json` sidecar. */
	schemasFor(name: string): NodeSchemas | undefined {
		return this.schemas.get(name);
	}

	clear(): void {
		this.byName.clear();
		this.schemas.clear();
	}

	/** Remove every signature that originated from the given document. */
	removeSource(uri: vscode.Uri): void {
		for (const [name, sig] of this.byName) {
			if (sig.source.toString() === uri.toString()) {
				this.byName.delete(name);
			}
		}
	}

	/** Parse a `.flow.d` document and merge its declarations into the registry. */
	ingest(uri: vscode.Uri, text: string): void {
		this.removeSource(uri);
		for (const sig of parseDeclarations(uri, text)) {
			this.byName.set(sig.name, sig);
		}
	}

	/** Merge a `.flow.schemas.json` sidecar (name -> per-pin JSON Schema strings). */
	ingestSchemas(text: string): void {
		let parsed: Record<string, NodeSchemas>;
		try {
			parsed = JSON.parse(text) as Record<string, NodeSchemas>;
		} catch {
			return;
		}
		for (const [name, schemas] of Object.entries(parsed)) {
			if (schemas && typeof schemas === "object") {
				this.schemas.set(name, schemas);
			}
		}
	}
}

export function parseDeclarations(
	uri: vscode.Uri,
	text: string,
): NodeSignature[] {
	const out: NodeSignature[] = [];
	const seen = new Set<number>();

	DECLARE_RE.lastIndex = 0;
	let m: RegExpExecArray | null;
	while ((m = DECLARE_RE.exec(text)) !== null) {
		const [, jsdoc, name, paramsRaw, returnRaw] = m;
		const nameOffset = m.index + m[0].indexOf(name, jsdoc.length);
		seen.add(nameOffset);
		out.push(
			buildSignature(uri, text, name, nameOffset, paramsRaw, returnRaw, jsdoc),
		);
	}

	// Declarations without a preceding JSDoc block.
	DECLARE_NO_DOC_RE.lastIndex = 0;
	while ((m = DECLARE_NO_DOC_RE.exec(text)) !== null) {
		const [, name, paramsRaw, returnRaw] = m;
		const nameOffset = m.index + m[0].indexOf(name);
		if (seen.has(nameOffset)) {
			continue;
		}
		out.push(
			buildSignature(uri, text, name, nameOffset, paramsRaw, returnRaw, ""),
		);
	}

	return out;
}

function buildSignature(
	uri: vscode.Uri,
	text: string,
	name: string,
	nameOffset: number,
	paramsRaw: string,
	returnRaw: string,
	jsdoc: string,
): NodeSignature {
	const doc = parseJsDoc(jsdoc);
	const params = parseParams(paramsRaw, doc.params);
	const returnText = returnRaw.trim();
	const returns = parseReturns(returnText, doc.returns);
	const nameRange = offsetRange(text, nameOffset, name.length);
	return {
		name,
		params,
		returns,
		returnText,
		description: doc.description,
		impure: doc.impure,
		source: uri,
		nameRange,
	};
}

interface JsDocInfo {
	description?: string;
	params: Map<string, { optional: boolean; doc: string }>;
	returns: Array<{ name?: string; doc: string }>;
	impure: boolean;
}

function parseJsDoc(jsdoc: string): JsDocInfo {
	const params = new Map<string, { optional: boolean; doc: string }>();
	const returns: Array<{ name?: string; doc: string }> = [];
	const descriptionLines: string[] = [];
	let impure = false;

	for (const rawLine of jsdoc.split("\n")) {
		const line = rawLine.replace(/^\s*\*\s?/, "").trim();
		if (line.length === 0) {
			continue;
		}
		if (line.startsWith("@param")) {
			const match =
				/^@param\s+([A-Za-z_$][\w$]*)\s*(\(optional\))?\s*(?:[—-]\s*)?(.*)$/.exec(
					line,
				);
			if (match) {
				params.set(match[1], {
					optional: Boolean(match[2]),
					doc: match[3].trim(),
				});
			}
		} else if (line.startsWith("@returns") || line.startsWith("@return")) {
			const match = /^@returns?\s+([A-Za-z_$][\w$]*)?\s*(?:[—-]\s*)?(.*)$/.exec(
				line,
			);
			if (match) {
				returns.push({ name: match[1], doc: match[2].trim() });
			}
		} else if (line.startsWith("@impure")) {
			impure = true;
		} else if (line.startsWith("@pure")) {
			impure = false;
		} else if (!line.startsWith("@")) {
			descriptionLines.push(line);
		}
	}

	return {
		description: descriptionLines.join(" ").trim() || undefined,
		params,
		returns,
		impure,
	};
}

function parseParams(
	raw: string,
	docs: Map<string, { optional: boolean; doc: string }>,
): ParamDoc[] {
	const inner = raw.trim().replace(/^\{/, "").replace(/\}$/, "").trim();
	if (inner.length === 0) {
		return [];
	}
	const out: ParamDoc[] = [];
	for (const part of splitTopLevel(inner)) {
		const match = /^([A-Za-z_$][\w$]*)(\?)?\s*:\s*(.+)$/.exec(part.trim());
		if (!match) {
			continue;
		}
		const docEntry = docs.get(match[1]);
		out.push({
			name: match[1],
			optional: Boolean(match[2]) || (docEntry?.optional ?? false),
			type: match[3].trim(),
			doc: docEntry?.doc,
		});
	}
	return out;
}

function parseReturns(
	returnText: string,
	docs: Array<{ name?: string; doc: string }>,
): ReturnDoc[] {
	if (returnText === "void") {
		return [];
	}
	if (returnText.startsWith("{") && returnText.endsWith("}")) {
		const inner = returnText.slice(1, -1).trim();
		const out: ReturnDoc[] = [];
		for (const part of splitTopLevel(inner)) {
			const match = /^([A-Za-z_$][\w$]*)\s*:\s*(.+)$/.exec(part.trim());
			if (!match) {
				continue;
			}
			const doc = docs.find((d) => d.name === match[1]);
			out.push({ name: match[1], type: match[2].trim(), doc: doc?.doc });
		}
		return out;
	}
	return [{ name: docs[0]?.name, type: returnText, doc: docs[0]?.doc }];
}

/** Split a comma-separated list, ignoring commas nested inside brackets. */
function splitTopLevel(input: string): string[] {
	const parts: string[] = [];
	let depth = 0;
	let current = "";
	for (const ch of input) {
		if (ch === "{" || ch === "[" || ch === "(" || ch === "<") {
			depth++;
		} else if (ch === "}" || ch === "]" || ch === ")" || ch === ">") {
			depth--;
		}
		if (ch === "," && depth === 0) {
			parts.push(current);
			current = "";
		} else {
			current += ch;
		}
	}
	if (current.trim().length > 0) {
		parts.push(current);
	}
	return parts;
}

function offsetRange(
	text: string,
	offset: number,
	length: number,
): vscode.Range {
	const start = positionAt(text, offset);
	const end = positionAt(text, offset + length);
	return new vscode.Range(start, end);
}

function positionAt(text: string, offset: number): vscode.Position {
	let line = 0;
	let last = 0;
	for (let i = 0; i < offset; i++) {
		if (text.charCodeAt(i) === 10) {
			line++;
			last = i + 1;
		}
	}
	return new vscode.Position(line, offset - last);
}

/** Render a signature label such as `floatAdd(float1: float, float2: float): float`. */
export function signatureLabel(sig: NodeSignature): string {
	const params = sig.params
		.map((p) => `${p.name}${p.optional ? "?" : ""}: ${p.type}`)
		.join(", ");
	return `${sig.name}({ ${params} }): ${sig.returnText}`;
}

/** Build a Markdown documentation block for a signature. */
export function signatureMarkdown(sig: NodeSignature): vscode.MarkdownString {
	const md = new vscode.MarkdownString();
	md.appendCodeblock(signatureLabel(sig), "typescript");
	if (sig.description) {
		md.appendMarkdown(`\n\n${sig.description}`);
	}
	if (sig.params.length > 0) {
		md.appendMarkdown("\n\n**Parameters**\n");
		for (const p of sig.params) {
			const opt = p.optional ? " _(optional)_" : "";
			const doc = p.doc ? ` — ${p.doc}` : "";
			md.appendMarkdown(`\n- \`${p.name}: ${p.type}\`${opt}${doc}`);
		}
	}
	if (sig.returns.length > 0) {
		md.appendMarkdown("\n\n**Returns**\n");
		for (const r of sig.returns) {
			const label = r.name ? `\`${r.name}: ${r.type}\`` : `\`${r.type}\``;
			const doc = r.doc ? ` — ${r.doc}` : "";
			md.appendMarkdown(`\n- ${label}${doc}`);
		}
	}
	if (sig.impure) {
		md.appendMarkdown("\n\n_Impure — has side effects / drives control flow._");
	}
	return md;
}
