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
	/** Display name: the qualified `ns::alias` spelling when known, else the legacy flat name. */
	readonly name: string;
	/** Catalog node type (`@node`), else the flat name. One registry entry per node type. */
	readonly nodeType: string;
	/** Legacy flat spelling (`stringContains`), accepted forever. */
	readonly flat: string;
	/** Namespace segments (`["ai", "ml"]`) when the declaration or `names.json` carries them. */
	readonly namespace?: readonly string[];
	/** Member name inside the namespace (`contains`). */
	readonly alias?: string;
	/** Name of the parameter bound by the receiver in method form (`x.alias()`). */
	readonly receiver?: string;
	/** Method class the receiver belongs to (`string`, `array`, a schema title). */
	readonly receiverClass?: string;
	/** Every data input, receiver included, in pin order (static call shape). */
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

/** One row of the generated `flow.d/names.json` snapshot. */
export interface NodeNames {
	readonly qualified: string;
	readonly namespace: string;
	readonly alias: string;
	readonly flat: string;
	readonly receiver: string | null;
	readonly class: string | null;
}

export interface NamespaceEntry {
	readonly path: readonly string[];
	readonly key: string;
	readonly members: Map<string, NodeSignature>;
	readonly children: Map<string, NamespaceEntry>;
}

/** Receivers of type `Generic` join every class. */
export const UNIVERSAL_CLASS = "universal";

const VALUE_CLASSES = new Set([
	"string",
	"int",
	"float",
	"bool",
	"array",
	"map",
	"set",
	"struct",
	"bytes",
	"path",
	"datetime",
]);

const IDENT = "[A-Za-z_$][\\w$]*";
const FUNCTION_RE = new RegExp(
	`(?:\\/\\*\\*([\\s\\S]*?)\\*\\/\\s*)?(?:declare\\s+)?function\\s+(${IDENT})\\s*\\(([^()]*(?:\\([^()]*\\)[^()]*)*)\\)\\s*:\\s*([^;]+);`,
	"g",
);
const NAMESPACE_HEAD_RE = new RegExp(
	`(?:declare\\s+)?namespace\\s+(${IDENT}(?:\\s*(?:\\.|::)\\s*${IDENT})*)\\s*\\{`,
	"g",
);

/** Mirrors `to_camel_case` in packages/ast/src/text.rs for pin names found in JSDoc tags. */
export function toCamelCase(input: string): string {
	let out = "";
	let upper = false;
	let first = true;
	for (const ch of input) {
		if (/[\p{L}\p{N}]/u.test(ch)) {
			if (first) {
				out += ch.toLowerCase();
				first = false;
			} else {
				out += upper ? ch.toUpperCase() : ch;
			}
			upper = false;
		} else if (!first) {
			upper = true;
		}
	}
	if (out.length === 0) {
		return "node";
	}
	return /^\d/.test(out) ? `_${out}` : out;
}

export function namespaceKey(path: readonly string[]): string {
	return path.join("::");
}

export function splitNamespace(namespace: string): string[] {
	return namespace
		.split(/::|\./)
		.map((segment) => segment.trim())
		.filter(Boolean);
}

/** Method class of a declared type text (`string`, `int[]`, `Map<string, T>`, `HttpResponse`). */
export function classOfTypeText(raw: string): string | undefined {
	const text = raw.trim();
	if (text.endsWith("[]")) {
		return "array";
	}
	if (/^Map\s*</.test(text)) {
		return "map";
	}
	if (/^Set\s*</.test(text)) {
		return "set";
	}
	switch (text) {
		case "string":
			return "string";
		case "int":
			return "int";
		case "float":
			return "float";
		case "bool":
			return "bool";
		case "Struct":
			return "struct";
		case "bytes":
		case "Byte":
			return "bytes";
		case "Path":
		case "PathBuf":
			return "path";
		case "Date":
			return "datetime";
		case "any":
		case "Generic":
		case "void":
		case "null":
		case "number":
			return undefined;
		default:
			return /^[A-Z][\w$]*$/.test(text) ? text : undefined;
	}
}

export function isTitledStructClass(cls: string): boolean {
	return !VALUE_CLASSES.has(cls) && cls !== UNIVERSAL_CLASS;
}

/**
 * Registry of all node signatures discovered from `.flow.d` files in the workspace, indexed by
 * every accepted spelling: the legacy flat name, the qualified `ns::alias` path and, for nodes
 * with a receiver, the method alias per class. Functions declared in a `.flow.d` are the
 * catalog nodes callable from `.flow` files.
 */
export class SignatureRegistry {
	private readonly byNodeType = new Map<string, NodeSignature>();
	private readonly byFlat = new Map<string, NodeSignature>();
	private readonly byQualified = new Map<string, NodeSignature>();
	private readonly namespaceEntries = new Map<string, NamespaceEntry>();
	private readonly methods = new Map<string, Map<string, NodeSignature[]>>();
	private readonly schemas = new Map<string, NodeSchemas>();
	private names = new Map<string, NodeNames>();

	get size(): number {
		return this.byNodeType.size;
	}

	all(): IterableIterator<NodeSignature> {
		return this.byNodeType.values();
	}

	/** Looks a node up by flat name, qualified path or node type. */
	get(name: string): NodeSignature | undefined {
		return (
			this.byQualified.get(name.replace(/\s*::\s*/g, "::")) ??
			this.byFlat.get(name) ??
			this.byNodeType.get(name)
		);
	}

	has(name: string): boolean {
		return this.get(name) !== undefined;
	}

	namespace(path: readonly string[]): NamespaceEntry | undefined {
		return this.namespaceEntries.get(namespaceKey(path));
	}

	namespaces(): IterableIterator<NamespaceEntry> {
		return this.namespaceEntries.values();
	}

	get namespaceCount(): number {
		return this.namespaceEntries.size;
	}

	get methodCount(): number {
		return this.methods.size;
	}

	/** Member `alias` of the namespace at `path`. */
	member(path: readonly string[], alias: string): NodeSignature | undefined {
		return this.namespace(path)?.members.get(alias);
	}

	/**
	 * Nodes callable as `value.alias(...)` for a receiver class (titled structs also accept
	 * plain `struct` methods; universal methods join every class; unknown class → every class).
	 */
	methodCandidates(cls: string | undefined, alias: string): NodeSignature[] {
		const out: NodeSignature[] = [];
		const push = (sigs: NodeSignature[] | undefined) => {
			for (const sig of sigs ?? []) {
				if (!out.includes(sig)) {
					out.push(sig);
				}
			}
		};
		if (cls) {
			push(this.methods.get(cls)?.get(alias));
			if (isTitledStructClass(cls)) {
				push(this.methods.get("struct")?.get(alias));
			}
			push(this.methods.get(UNIVERSAL_CLASS)?.get(alias));
			return out;
		}
		for (const table of this.methods.values()) {
			push(table.get(alias));
		}
		return out;
	}

	/** Every method of a class (plus struct fallbacks and universal methods), or of every class. */
	methodsOf(cls: string | undefined): Array<{ sig: NodeSignature; cls: string }> {
		const out: Array<{ sig: NodeSignature; cls: string }> = [];
		const seen = new Set<NodeSignature>();
		const pushAll = (group: string) => {
			for (const bucket of this.methods.get(group)?.values() ?? []) {
				for (const sig of bucket) {
					if (!seen.has(sig)) {
						seen.add(sig);
						out.push({ sig, cls: group });
					}
				}
			}
		};
		if (cls) {
			pushAll(cls);
			if (isTitledStructClass(cls)) {
				pushAll("struct");
			}
			pushAll(UNIVERSAL_CLASS);
			return out;
		}
		for (const group of this.methods.keys()) {
			pushAll(group);
		}
		return out;
	}

	/** Per-pin JSON Schema strings for a node, from a `.flow.schemas.json` sidecar keyed by node type or display name. */
	schemasFor(node: NodeSignature | string): NodeSchemas | undefined {
		const sig = typeof node === "string" ? this.get(node) : node;
		if (!sig) {
			return typeof node === "string" ? this.schemas.get(node) : undefined;
		}
		return (
			this.schemas.get(sig.nodeType) ??
			this.schemas.get(sig.flat) ??
			this.schemas.get(sig.name)
		);
	}

	clear(): void {
		this.byNodeType.clear();
		this.schemas.clear();
		this.rebuild();
	}

	/** Remove every signature that originated from the given document. */
	removeSource(uri: vscode.Uri): void {
		for (const [nodeType, sig] of this.byNodeType) {
			if (sig.source.toString() === uri.toString()) {
				this.byNodeType.delete(nodeType);
			}
		}
		this.rebuild();
	}

	/** Parse a `.flow.d` document and merge its declarations into the registry. */
	ingest(uri: vscode.Uri, text: string): void {
		for (const [nodeType, sig] of this.byNodeType) {
			if (sig.source.toString() === uri.toString()) {
				this.byNodeType.delete(nodeType);
			}
		}
		for (const sig of parseDeclarations(uri, text, this.names)) {
			this.byNodeType.set(sig.nodeType, sig);
		}
		this.rebuild();
	}

	/** Merge a `.flow.schemas.json` sidecar (node type or display name → per-pin JSON Schema strings). */
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

	/**
	 * Merge the generated `names.json` snapshot (node type → names). Legacy flat declarations
	 * gain their namespace, alias and receiver from it; declarations are re-enriched in place.
	 */
	ingestNames(text: string): void {
		let parsed: unknown;
		try {
			parsed = JSON.parse(text);
		} catch {
			return;
		}
		if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
			return;
		}
		const names = new Map<string, NodeNames>();
		for (const [nodeType, row] of Object.entries(parsed as Record<string, unknown>)) {
			if (!row || typeof row !== "object") {
				continue;
			}
			const entry = row as Record<string, unknown>;
			if (typeof entry.qualified !== "string" || typeof entry.flat !== "string") {
				continue;
			}
			names.set(nodeType, {
				qualified: entry.qualified,
				namespace: typeof entry.namespace === "string" ? entry.namespace : "",
				alias: typeof entry.alias === "string" ? entry.alias : "",
				flat: entry.flat,
				receiver: typeof entry.receiver === "string" ? entry.receiver : null,
				class: typeof entry.class === "string" ? entry.class : null,
			});
		}
		this.names = names;
		const enriched = [...this.byNodeType.values()].map((sig) =>
			enrichWithNames(sig, names),
		);
		this.byNodeType.clear();
		for (const sig of enriched) {
			this.byNodeType.set(sig.nodeType, sig);
		}
		this.rebuild();
	}

	private rebuild(): void {
		this.byFlat.clear();
		this.byQualified.clear();
		this.namespaceEntries.clear();
		this.methods.clear();
		for (const sig of this.byNodeType.values()) {
			if (!this.byFlat.has(sig.flat)) {
				this.byFlat.set(sig.flat, sig);
			}
			if (sig.namespace && sig.alias) {
				const qualified = `${namespaceKey(sig.namespace)}::${sig.alias}`;
				if (!this.byQualified.has(qualified)) {
					this.byQualified.set(qualified, sig);
					this.ensureNamespace(sig.namespace).members.set(sig.alias, sig);
				}
				if (sig.receiverClass) {
					let table = this.methods.get(sig.receiverClass);
					if (!table) {
						table = new Map();
						this.methods.set(sig.receiverClass, table);
					}
					const bucket = table.get(sig.alias) ?? [];
					if (!bucket.includes(sig)) {
						bucket.push(sig);
					}
					table.set(sig.alias, bucket);
				}
			}
		}
	}

	private ensureNamespace(path: readonly string[]): NamespaceEntry {
		let current: NamespaceEntry | undefined;
		for (let i = 1; i <= path.length; i++) {
			const prefix = path.slice(0, i);
			const key = namespaceKey(prefix);
			let entry = this.namespaceEntries.get(key);
			if (!entry) {
				entry = { path: prefix, key, members: new Map(), children: new Map() };
				this.namespaceEntries.set(key, entry);
				current?.children.set(prefix[prefix.length - 1], entry);
			}
			current = entry;
		}
		return current as NamespaceEntry;
	}
}

/** Fill namespace/alias/receiver from the names snapshot when the declaration lacks them. */
function enrichWithNames(
	sig: NodeSignature,
	names: ReadonlyMap<string, NodeNames>,
): NodeSignature {
	let row = names.get(sig.nodeType);
	if (!row) {
		for (const [nodeType, candidate] of names) {
			if (candidate.flat === sig.flat) {
				row = candidate;
				sig = { ...sig, nodeType };
				break;
			}
		}
	}
	if (!row) {
		return sig;
	}
	const namespace = sig.namespace ?? splitNamespace(row.namespace);
	const alias = sig.alias ?? row.alias;
	let receiver = sig.receiver;
	let receiverClass = sig.receiverClass;
	if (!receiver && row.receiver) {
		const paramName = toCamelCase(row.receiver);
		const param = sig.params.find((p) => p.name === paramName);
		if (param) {
			receiver = param.name;
			receiverClass = row.class ?? classOfTypeText(param.type) ?? UNIVERSAL_CLASS;
		}
	}
	return {
		...sig,
		name:
			namespace.length > 0 && alias ? `${namespaceKey(namespace)}::${alias}` : sig.name,
		namespace: namespace.length > 0 ? namespace : undefined,
		alias: namespace.length > 0 ? alias : undefined,
		receiver,
		receiverClass,
	};
}

interface Block {
	readonly path: string[];
	readonly start: number;
	readonly end: number;
}

/** Find `declare namespace a { … }` / nested `namespace b { … }` blocks with their text spans. */
function namespaceBlocks(text: string, base: string[], from: number, to: number): Block[] {
	const out: Block[] = [];
	NAMESPACE_HEAD_RE.lastIndex = from;
	let m: RegExpExecArray | null;
	while ((m = NAMESPACE_HEAD_RE.exec(text)) !== null && m.index < to) {
		const bodyStart = m.index + m[0].length;
		const bodyEnd = matchingBrace(text, bodyStart - 1);
		if (bodyEnd < 0 || bodyEnd > to) {
			break;
		}
		const path = [...base, ...splitNamespace(m[1])];
		out.push({ path, start: bodyStart, end: bodyEnd });
		out.push(...namespaceBlocks(text, path, bodyStart, bodyEnd));
		NAMESPACE_HEAD_RE.lastIndex = bodyEnd + 1;
	}
	return out;
}

/** Offset of the `}` matching the `{` at `open`, skipping strings and comments. */
function matchingBrace(text: string, open: number): number {
	let depth = 0;
	let i = open;
	while (i < text.length) {
		const ch = text[i];
		if (ch === "/" && text[i + 1] === "*") {
			const close = text.indexOf("*/", i + 2);
			i = close < 0 ? text.length : close + 2;
			continue;
		}
		if (ch === "/" && text[i + 1] === "/") {
			const nl = text.indexOf("\n", i);
			i = nl < 0 ? text.length : nl + 1;
			continue;
		}
		if (ch === '"' || ch === "'" || ch === "`") {
			let j = i + 1;
			while (j < text.length && text[j] !== ch) {
				if (text[j] === "\\") {
					j++;
				}
				j++;
			}
			i = j + 1;
			continue;
		}
		if (ch === "{") {
			depth++;
		} else if (ch === "}") {
			depth--;
			if (depth === 0) {
				return i;
			}
		}
		i++;
	}
	return -1;
}

/** Innermost namespace block containing `offset`, if any. */
function blockAt(blocks: readonly Block[], offset: number): Block | undefined {
	let best: Block | undefined;
	for (const block of blocks) {
		if (offset >= block.start && offset < block.end) {
			if (!best || block.start >= best.start) {
				best = block;
			}
		}
	}
	return best;
}

/**
 * Parse every declaration in a `.flow.d` document: legacy flat `declare function name(...)`
 * lines and the namespaced form `declare namespace a { function alias(this: T, { … }): R; }`
 * (nested namespaces allowed; JSDoc `@node`, `@receiver`, `@alias` tags carry the node type,
 * the receiver pin and the legacy flat name).
 */
export function parseDeclarations(
	uri: vscode.Uri,
	text: string,
	names: ReadonlyMap<string, NodeNames> = new Map(),
): NodeSignature[] {
	const out: NodeSignature[] = [];
	const blocks = namespaceBlocks(text, [], 0, text.length);

	FUNCTION_RE.lastIndex = 0;
	let m: RegExpExecArray | null;
	while ((m = FUNCTION_RE.exec(text)) !== null) {
		const [whole, jsdoc = "", name, paramsRaw, returnRaw] = m;
		const nameOffset = m.index + whole.lastIndexOf(`function ${name}`) + "function ".length;
		const block = blockAt(blocks, m.index);
		out.push(
			enrichWithNames(
				buildSignature(uri, text, name, nameOffset, paramsRaw, returnRaw, jsdoc, block?.path),
				names,
			),
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
	namespacePath: string[] | undefined,
): NodeSignature {
	const doc = parseJsDoc(jsdoc);
	const parsed = parseParams(paramsRaw, doc.params);
	const returnText = returnRaw.trim();
	const returns = parseReturns(returnText, doc.returns);
	const nameRange = offsetRange(text, nameOffset, name.length);
	const namespace = namespacePath && namespacePath.length > 0 ? namespacePath : undefined;
	const alias = namespace ? name : undefined;
	const flat = namespace ? (doc.alias ?? toCamelCase(doc.node ?? name)) : name;
	const nodeType = doc.node ?? flat;

	let params = parsed.params;
	let receiver: string | undefined;
	let receiverClass: string | undefined;
	if (parsed.thisType !== undefined) {
		// `this: T` — the receiver pin is not repeated in the object; synthesise it first.
		const receiverName = toCamelCase(doc.receiver ?? "value");
		params = [
			{
				name: receiverName,
				type: parsed.thisType,
				optional: false,
				doc: doc.params.get(receiverName)?.doc,
			},
			...parsed.params.filter((p) => p.name !== receiverName),
		];
		receiver = receiverName;
		receiverClass = classOfTypeText(parsed.thisType) ?? UNIVERSAL_CLASS;
	} else if (doc.receiver) {
		const receiverName = toCamelCase(doc.receiver);
		const param = params.find((p) => p.name === receiverName);
		if (param) {
			receiver = param.name;
			receiverClass = classOfTypeText(param.type) ?? UNIVERSAL_CLASS;
		}
	}

	return {
		name: namespace ? `${namespaceKey(namespace)}::${name}` : name,
		nodeType,
		flat,
		namespace,
		alias,
		receiver,
		receiverClass,
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
	node?: string;
	receiver?: string;
	alias?: string;
}

function parseJsDoc(jsdoc: string): JsDocInfo {
	const params = new Map<string, { optional: boolean; doc: string }>();
	const returns: Array<{ name?: string; doc: string }> = [];
	const descriptionLines: string[] = [];
	let impure = false;
	let node: string | undefined;
	let receiver: string | undefined;
	let alias: string | undefined;

	// Tags may share a line (`@node string_contains @receiver string @alias stringContains`).
	const lines = jsdoc
		.split("\n")
		.map((rawLine) => rawLine.replace(/^\s*\*\s?/, "").trim())
		.flatMap((line) =>
			line.split(/\s+(?=@(?:node|receiver|alias|impure|pure)\b)/).map((part) => part.trim()),
		);

	for (const line of lines) {
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
		} else if (line.startsWith("@node")) {
			node = /^@node\s+(\S+)/.exec(line)?.[1];
		} else if (line.startsWith("@receiver")) {
			receiver = /^@receiver\s+(\S+)/.exec(line)?.[1];
		} else if (line.startsWith("@alias")) {
			alias = /^@alias\s+(\S+)/.exec(line)?.[1];
		} else if (!line.startsWith("@")) {
			descriptionLines.push(line);
		}
	}

	return {
		description: descriptionLines.join(" ").trim() || undefined,
		params,
		returns,
		impure,
		node,
		receiver,
		alias,
	};
}

function parseParams(
	raw: string,
	docs: Map<string, { optional: boolean; doc: string }>,
): { params: ParamDoc[]; thisType?: string } {
	let thisType: string | undefined;
	let objectRaw = raw.trim();
	// `this: T, { … }` / `this: T` — peel the receiver parameter off first.
	const thisMatch = /^this\s*:\s*([^,{]+?)\s*(?:,\s*([\s\S]*))?$/.exec(objectRaw);
	if (thisMatch) {
		thisType = thisMatch[1].trim();
		objectRaw = (thisMatch[2] ?? "").trim();
	}
	const inner = objectRaw.replace(/^\{/, "").replace(/\}$/, "").trim();
	if (inner.length === 0) {
		return { params: [], thisType };
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
	return { params: out, thisType };
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

/** Parameters bound by name in method form (the receiver is already bound). */
export function methodParams(sig: NodeSignature): ParamDoc[] {
	return sig.receiver ? sig.params.filter((p) => p.name !== sig.receiver) : sig.params;
}

/** Render a signature label such as `float::add({ float1: float, float2: float }): float`. */
export function signatureLabel(sig: NodeSignature, method = false): string {
	const asMethod = method && sig.receiver && sig.alias;
	const head = asMethod ? `${sig.receiverClass ?? "value"}.${sig.alias}` : sig.name;
	const params = (asMethod ? methodParams(sig) : sig.params)
		.map((p) => `${p.name}${p.optional ? "?" : ""}: ${p.type}`)
		.join(", ");
	return params.length > 0
		? `${head}({ ${params} }): ${sig.returnText}`
		: `${head}(): ${sig.returnText}`;
}

/** Build a Markdown documentation block for a signature. */
export function signatureMarkdown(sig: NodeSignature): vscode.MarkdownString {
	const md = new vscode.MarkdownString();
	md.appendCodeblock(signatureLabel(sig), "typescript");
	if (sig.description) {
		md.appendMarkdown(`\n\n${sig.description}`);
	}
	const spellings: string[] = [];
	if (sig.receiver && sig.alias) {
		const rest = methodParams(sig);
		const call =
			rest.length === 0
				? `x.${sig.alias}()`
				: rest.length === 1
					? `x.${sig.alias}(${rest[0].name})`
					: `x.${sig.alias}({ ${rest.map((p) => p.name).join(", ")} })`;
		spellings.push(`method form \`${call}\` on \`${sig.receiverClass ?? "any"}\``);
	}
	if (sig.namespace) {
		spellings.push(`legacy \`${sig.flat}(…)\``);
	}
	if (spellings.length > 0) {
		md.appendMarkdown(`\n\nAlso callable as ${spellings.join(", ")}.`);
	}
	if (sig.params.length > 0) {
		md.appendMarkdown("\n\n**Parameters**\n");
		for (const p of sig.params) {
			const opt = p.optional ? " _(optional)_" : "";
			const recv = p.name === sig.receiver ? " _(receiver)_" : "";
			const doc = p.doc ? ` — ${p.doc}` : "";
			md.appendMarkdown(`\n- \`${p.name}: ${p.type}\`${opt}${recv}${doc}`);
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
