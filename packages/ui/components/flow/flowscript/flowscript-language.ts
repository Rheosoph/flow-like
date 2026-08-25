import type { Monaco } from "@monaco-editor/react";
// Type-only: the board scope is defined next to the module helpers that build it, and erases at
// build time, so the language worker never pulls the board/layer modules in.
import type { FlowScriptBoardScope } from "../../../lib/flow-modules";
import {
	type FlowScriptNamesTable,
	getFlowScriptNamesTable,
	loadFlowScriptNamesTable,
	namespaceSegments,
	resolveFlowScriptNames,
} from "../../../lib/flowscript/names";
import type { INode, IPin } from "../../../lib/schema/flow/node";
import {
	IPinType,
	IValueType,
	IVariableType,
} from "../../../lib/schema/flow/pin";
import { registerFlowScriptFeatureProviders } from "./flowscript-language-features";
import {
	type CancellationTokenLike,
	requestFlowScriptWorkerEnvDoc,
} from "./flowscript-worker-client";

export type { FlowScriptBoardScope };

export const FLOWSCRIPT_LANGUAGE_ID = "flowscript";
export const FLOWSCRIPT_THEME_DARK = "flowscript-dark";
export const FLOWSCRIPT_THEME_LIGHT = "flowscript-light";
export const FLOWSCRIPT_DIAGNOSTIC_OWNER = "flowscript-client";

const STORAGE_KEYWORDS = [
	"const",
	"let",
	"function",
	"interface",
	"struct",
	"event",
	"declare",
	"use",
];

const CONTROL_KEYWORDS = [
	"if",
	"else",
	"for",
	"of",
	"in",
	"as",
	"return",
	"while",
	"break",
	"continue",
];

const TYPE_KEYWORDS = [
	"string",
	"int",
	"float",
	"bool",
	"void",
	"any",
	"Date",
	"Path",
	"PathBuf",
	"Struct",
	"Byte",
	"bytes",
	"Generic",
	"Map",
	"Set",
];

const CONSTANTS = ["true", "false", "null"];

/**
 * `module <name> { … }` groups a board's sections into a namespace. It is a keyword in header
 * position only, so it is deliberately NOT in `STORAGE_KEYWORDS` (which the tokenizer colors
 * wherever the word appears); Monarch matches the header shape instead.
 */
export const MODULE_KEYWORD = "module";

/**
 * `detached { … }` holds the execution chains lowering found unreachable; the block names no node,
 * so its header is the bare word before the brace. Like `module` it is a keyword in header
 * position only — `detached(…) { }` is still an event named `detached` — so it is deliberately NOT
 * in `STORAGE_KEYWORDS`; Monarch matches the header shape instead.
 */
export const DETACHED_KEYWORD = "detached";

export const KEYWORD_SET = new Set([
	...STORAGE_KEYWORDS,
	...CONTROL_KEYWORDS,
	MODULE_KEYWORD,
	DETACHED_KEYWORD,
]);

/** Every word with reserved meaning: keywords, type names and literal constants. */
export const RESERVED_WORDS: ReadonlySet<string> = new Set([
	...STORAGE_KEYWORDS,
	...CONTROL_KEYWORDS,
	...TYPE_KEYWORDS,
	...CONSTANTS,
	MODULE_KEYWORD,
	DETACHED_KEYWORD,
]);

/** Method class of receivers whose pin type is `Generic`: listed for every class. */
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

const IDENT_SRC = "[A-Za-z_$][\\w$]*";
const PATH_SRC = `(?:${IDENT_SRC}\\s*::\\s*)*${IDENT_SRC}`;
const IDENT_RE = new RegExp(`^${IDENT_SRC}$`);
const HEAD_RE = new RegExp(`^${PATH_SRC}`);
const PATH_SPLIT_RE = /\s*::\s*/;

/** Mirrors `to_camel_case` in packages/ast/src/text.rs so completions match rendered node names. */
export function toFlowScriptIdentifier(input: string): string {
	let out = "";
	let upcomingUpper = false;
	let first = true;
	for (const ch of input) {
		// Unicode letters/digits, mirroring Rust `char::is_alphanumeric` in `to_camel_case`.
		if (/[\p{L}\p{N}]/u.test(ch)) {
			if (first) {
				out += ch.toLowerCase();
				first = false;
			} else if (upcomingUpper) {
				out += ch.toUpperCase();
			} else {
				out += ch;
			}
			upcomingUpper = false;
		} else if (!first) {
			upcomingUpper = true;
		}
	}
	if (out.length === 0) return "node";
	return /^\d/.test(out) ? `_${out}` : out;
}

/** Mirrors `variable_type_base` in packages/core/src/flow/ast/types.rs. */
function variableTypeBase(dataType: IVariableType): string {
	switch (dataType) {
		case IVariableType.Execution:
			return "exec";
		case IVariableType.String:
			return "string";
		case IVariableType.Integer:
			return "int";
		case IVariableType.Float:
			return "float";
		case IVariableType.Boolean:
			return "bool";
		case IVariableType.Date:
			return "Date";
		case IVariableType.PathBuf:
			return "Path";
		case IVariableType.Generic:
			return "any";
		case IVariableType.Struct:
			return "Struct";
		case IVariableType.Byte:
			return "bytes";
		default:
			return "any";
	}
}

/** Mirrors `render_type_ref` in packages/ast/src/render.rs. */
function pinTypeString(pin: IPin): string {
	const base = variableTypeBase(pin.data_type);
	switch (pin.value_type) {
		case IValueType.Array:
			return `${base}[]`;
		case IValueType.HashMap:
			return `Map<string, ${base}>`;
		case IValueType.HashSet:
			return `Set<${base}>`;
		default:
			return base;
	}
}

/** Cheap root-title extraction from a JSON-schema string without parsing the whole payload. */
function schemaTitle(schema?: string | null): string | undefined {
	if (!schema) return undefined;
	const match = /"title"\s*:\s*"([^"]+)"/.exec(schema);
	return match?.[1];
}

/**
 * Method class of a value: the value-type namespace its type belongs to (`string`, `array`, …),
 * the schema title for a titled struct, or `undefined` for `Generic`/unknown. Mirrors
 * `receiver_class_of` in packages/ast/src/naming.rs.
 */
export function methodClassFor(
	dataType: IVariableType | undefined,
	container: IValueType | undefined,
	title?: string,
): string | undefined {
	switch (container) {
		case IValueType.Array:
			return "array";
		case IValueType.HashMap:
			return "map";
		case IValueType.HashSet:
			return "set";
		default:
			break;
	}
	switch (dataType) {
		case IVariableType.String:
			return "string";
		case IVariableType.Integer:
			return "int";
		case IVariableType.Float:
			return "float";
		case IVariableType.Boolean:
			return "bool";
		case IVariableType.Struct:
			return title ?? "struct";
		case IVariableType.Date:
			return "datetime";
		case IVariableType.Byte:
			return "bytes";
		case IVariableType.PathBuf:
			return "path";
		default:
			return undefined;
	}
}

export interface FlowScriptArg {
	name: string;
	/** Raw pin name as declared on the node (`format_string`). */
	rawName: string;
	friendlyName: string;
	description: string;
	typeString: string;
	dataType: IVariableType;
	container: IValueType;
	schemaTitle?: string;
	schema?: string;
	optional: boolean;
	enumValues?: string[];
	sensitive: boolean;
}

export interface FlowScriptOutput {
	name: string;
	rawName: string;
	typeString: string;
	description: string;
	dataType: IVariableType;
	container: IValueType;
	schemaTitle?: string;
	schema?: string;
}

export interface FlowScriptNodeInfo {
	/** Legacy flat spelling (`stringTrim`), accepted forever. */
	identifier: string;
	nodeType: string;
	/** `string::trim` when the node has a namespace. */
	qualified?: string;
	namespace?: string[];
	alias?: string;
	/** The argument bound by the receiver in method form (`s` in `s.trim()`). */
	receiver?: FlowScriptArg;
	/** Class the method form is callable on (`string`, `array`, a schema title, `universal`). */
	receiverClass?: string;
	friendlyName: string;
	description: string;
	docs?: string;
	category: string;
	impure: boolean;
	args: FlowScriptArg[];
	outputs: FlowScriptOutput[];
	/** Execution output pin names (camelCased): the arm labels of `bind { arm: { … } }` blocks. */
	execOutputs: string[];
	/** Output consumed when the call is used as a value (`x = node(...)`), if unambiguous. */
	defaultOutput?: FlowScriptOutput;
}

export interface FlowScriptNamespace {
	path: string[];
	/** `ai::ml` */
	key: string;
	members: Map<string, FlowScriptNodeInfo>;
	children: Map<string, FlowScriptNamespace>;
}

export interface FlowScriptIndex {
	/** Flat (legacy) name → node. */
	byName: Map<string, FlowScriptNodeInfo>;
	/** `ns::alias` → node. */
	byQualified: Map<string, FlowScriptNodeInfo>;
	/** Every namespace path (including intermediate ones) by its `::` key. */
	namespaces: Map<string, FlowScriptNamespace>;
	/** class → alias → candidate nodes callable as `value.alias(...)`. */
	methods: Map<string, Map<string, FlowScriptNodeInfo[]>>;
	names: string[];
}

export function namespaceKey(path: readonly string[]): string {
	return path.join("::");
}

function buildArg(pin: IPin): FlowScriptArg {
	const title =
		pin.data_type === IVariableType.Struct
			? schemaTitle(pin.schema)
			: undefined;
	const typeString = title
		? pinTypeString(pin).replace("Struct", title)
		: pinTypeString(pin);
	return {
		name: toFlowScriptIdentifier(pin.name),
		rawName: pin.name,
		friendlyName: pin.friendly_name || pin.name,
		description: pin.description ?? "",
		typeString,
		dataType: pin.data_type,
		container: pin.value_type,
		schemaTitle: title,
		schema: pin.schema ?? undefined,
		optional: pin.default_value != null,
		enumValues: pin.options?.valid_values ?? undefined,
		sensitive: pin.options?.sensitive === true,
	};
}

/** Mirrors `default_metadata_output_pin` in packages/core/src/flow/ast/reconcile.rs. */
const DEFAULT_OUTPUT_NAMES = new Set([
	"result",
	"value",
	"output",
	"out",
	"batch",
]);

function buildNodeInfo(
	node: INode,
	names?: FlowScriptNamesTable,
): FlowScriptNodeInfo {
	const pins = Object.values(node.pins);
	const args = pins
		.filter(
			(pin) =>
				pin.pin_type === IPinType.Input &&
				pin.data_type !== IVariableType.Execution,
		)
		.sort((a, b) => a.index - b.index)
		.map(buildArg);
	const outputs = pins
		.filter(
			(pin) =>
				pin.pin_type === IPinType.Output &&
				pin.data_type !== IVariableType.Execution,
		)
		.sort((a, b) => a.index - b.index)
		.map((pin) => ({
			name: toFlowScriptIdentifier(pin.name),
			rawName: pin.name,
			typeString: pinTypeString(pin),
			description: pin.description ?? "",
			dataType: pin.data_type,
			container: pin.value_type,
			schemaTitle: schemaTitle(pin.schema),
			schema: pin.schema ?? undefined,
		}));
	const impure = pins.some((pin) => pin.data_type === IVariableType.Execution);
	const execOutputs = pins
		.filter(
			(pin) =>
				pin.pin_type === IPinType.Output &&
				pin.data_type === IVariableType.Execution,
		)
		.sort((a, b) => a.index - b.index)
		.map((pin) => toFlowScriptIdentifier(pin.name));
	const resolved = resolveFlowScriptNames(node, names);
	const receiver = resolved?.receiver
		? args.find((arg) => arg.rawName === resolved.receiver)
		: undefined;
	const receiverClass = receiver
		? (resolved?.class ??
			methodClassFor(
				receiver.dataType,
				receiver.container,
				receiver.schemaTitle,
			) ??
			UNIVERSAL_CLASS)
		: undefined;
	const defaultOutput =
		outputs.length === 1
			? outputs[0]
			: outputs.find((out) => DEFAULT_OUTPUT_NAMES.has(out.rawName));
	return {
		identifier: resolved?.flat || toFlowScriptIdentifier(node.name),
		nodeType: node.name,
		qualified: resolved?.qualified,
		namespace: resolved ? [...resolved.namespace] : undefined,
		alias: resolved?.alias,
		receiver,
		receiverClass,
		friendlyName: node.friendly_name || node.name,
		description: node.description ?? "",
		docs: node.docs ?? undefined,
		category: node.category ?? "",
		impure,
		args,
		outputs,
		execOutputs,
		defaultOutput,
	};
}

function ensureNamespace(
	namespaces: Map<string, FlowScriptNamespace>,
	path: string[],
): FlowScriptNamespace {
	let current: FlowScriptNamespace | undefined;
	for (let i = 1; i <= path.length; i++) {
		const prefix = path.slice(0, i);
		const key = namespaceKey(prefix);
		let ns = namespaces.get(key);
		if (!ns) {
			ns = { path: prefix, key, members: new Map(), children: new Map() };
			namespaces.set(key, ns);
			current?.children.set(prefix[prefix.length - 1], ns);
		}
		current = ns;
	}
	return current as FlowScriptNamespace;
}

export function buildFlowScriptIndex(
	nodes: INode[],
	names?: FlowScriptNamesTable,
): FlowScriptIndex {
	const byName = new Map<string, FlowScriptNodeInfo>();
	const byQualified = new Map<string, FlowScriptNodeInfo>();
	const namespaces = new Map<string, FlowScriptNamespace>();
	const methods = new Map<string, Map<string, FlowScriptNodeInfo[]>>();
	for (const node of nodes) {
		const info = buildNodeInfo(node, names);
		if (!byName.has(info.identifier)) byName.set(info.identifier, info);
		if (info.qualified && info.namespace && info.alias) {
			if (!byQualified.has(info.qualified)) {
				byQualified.set(info.qualified, info);
				ensureNamespace(namespaces, info.namespace).members.set(
					info.alias,
					info,
				);
			}
			if (info.receiverClass) {
				let table = methods.get(info.receiverClass);
				if (!table) {
					table = new Map();
					methods.set(info.receiverClass, table);
				}
				const bucket = table.get(info.alias) ?? [];
				if (!bucket.includes(info)) bucket.push(info);
				table.set(info.alias, bucket);
			}
		}
	}
	return {
		byName,
		byQualified,
		namespaces,
		methods,
		names: [...byName.keys()],
	};
}

let cachedNodes: INode[] | undefined;
let cachedNames: FlowScriptNamesTable | undefined;
let cachedIndex: FlowScriptIndex | undefined;

/**
 * Memoized on catalog identity and on the generated names snapshot, so it only rebuilds when the
 * catalog prop changes or the snapshot arrives (it is loaded lazily on first use).
 */
export function getFlowScriptIndex(
	nodes: INode[] | undefined,
): FlowScriptIndex {
	const names = getFlowScriptNamesTable();
	if (!names) loadFlowScriptNamesTable().catch(() => undefined);
	if (nodes === cachedNodes && names === cachedNames && cachedIndex)
		return cachedIndex;
	cachedIndex = buildFlowScriptIndex(nodes ?? [], names);
	cachedNodes = nodes;
	cachedNames = names;
	return cachedIndex;
}

/** Root segments of the board's modules — the names a `::` path may legitimately start with. */
export function boardModuleRoots(
	board: FlowScriptBoardScope | undefined,
): ReadonlySet<string> {
	const roots = new Set<string>();
	for (const key of board?.modules ?? []) {
		const root = key.split("::")[0]?.trim();
		if (root) roots.add(root);
	}
	return roots;
}

/** Every function/event the board declares, in any file. */
export function boardFunctionNames(
	board: FlowScriptBoardScope | undefined,
): ReadonlySet<string> {
	const names = new Set<string>();
	for (const list of Object.values(board?.functionsByModule ?? {})) {
		for (const name of list) names.add(name);
	}
	return names;
}

/**
 * Top-level catalog namespaces (`string`, `ai`, `hash`, …). A module named after one would make
 * every qualified call inside it ambiguous, so module-name validation reserves them.
 *
 * One pass over the nodes: it neither builds the catalog index nor forces the generated names
 * snapshot to load, so a board that never opens FlowScript pays nothing. Until the snapshot is
 * in, only nodes carrying an explicit namespace contribute — call
 * {@link loadFlowScriptNamesTable} and recompute when it resolves to get the complete set.
 */
export function catalogNamespaceRoots(nodes: INode[] | undefined): string[] {
	const names = getFlowScriptNamesTable();
	const roots = new Set<string>();
	for (const node of nodes ?? []) {
		const namespace = node.namespace?.trim() || names?.[node.name]?.namespace;
		const root = namespace ? namespaceSegments(namespace)[0] : undefined;
		if (root) roots.add(root);
	}
	return [...roots];
}

export function displayName(info: FlowScriptNodeInfo): string {
	return info.qualified ?? info.identifier;
}

function argSignature(arg: FlowScriptArg): string {
	return `${arg.name}${arg.optional ? "?" : ""}: ${arg.typeString}`;
}

export function methodParams(info: FlowScriptNodeInfo): FlowScriptArg[] {
	return info.receiver
		? info.args.filter((arg) => arg !== info.receiver)
		: info.args;
}

export function renderSignature(
	info: FlowScriptNodeInfo,
	method = false,
): string {
	const asMethod = method && info.receiver && info.alias;
	const head = asMethod
		? `${info.receiverClass ?? "value"}.${info.alias}`
		: displayName(info);
	const params = asMethod ? methodParams(info) : info.args;
	if (params.length === 0) return `${head}()`;
	return `${head}({ ${params.map(argSignature).join(", ")} })`;
}

function methodFormExample(info: FlowScriptNodeInfo): string | undefined {
	if (!info.receiver || !info.alias) return undefined;
	const rest = methodParams(info);
	const receiverName = info.receiverClass === UNIVERSAL_CLASS ? "value" : "x";
	if (rest.length === 0) return `${receiverName}.${info.alias}()`;
	if (rest.length === 1)
		return `${receiverName}.${info.alias}(${rest[0].name})`;
	return `${receiverName}.${info.alias}({ ${rest.map((arg) => arg.name).join(", ")} })`;
}

export function nodeHoverMarkdown(info: FlowScriptNodeInfo): string {
	const lines: string[] = [];
	lines.push(`\`\`\`flowscript\n${renderSignature(info)}\n\`\`\``);
	const meta = [info.category, info.impure ? "impure" : "pure"]
		.filter(Boolean)
		.join(" · ");
	if (meta) lines.push(`_${meta}_`);
	const spellings: string[] = [];
	const method = methodFormExample(info);
	if (method)
		spellings.push(
			`method form \`${method}\` on \`${info.receiverClass ?? "any"}\``,
		);
	if (info.qualified) spellings.push(`legacy \`${info.identifier}(…)\``);
	if (spellings.length > 0)
		lines.push(`Also callable as ${spellings.join(", ")}.`);
	if (info.description) lines.push(info.description);
	if (info.docs && info.docs !== info.description) lines.push(info.docs);
	if (info.outputs.length > 0) {
		lines.push(
			`**Returns:** ${info.outputs
				.map((out) => `\`${out.name}: ${out.typeString}\``)
				.join(", ")}`,
		);
	}
	return lines.join("\n\n");
}

function argHoverMarkdown(
	info: FlowScriptNodeInfo,
	arg: FlowScriptArg,
): string {
	const lines: string[] = [];
	lines.push(`\`${argSignature(arg)}\` — argument of \`${displayName(info)}\``);
	if (arg.friendlyName && arg.friendlyName !== arg.name)
		lines.push(`**${arg.friendlyName}**`);
	if (arg.description) lines.push(arg.description);
	if (arg.enumValues && arg.enumValues.length > 0)
		lines.push(
			`Allowed: ${arg.enumValues.map((value) => `\`${value}\``).join(", ")}`,
		);
	if (arg === info.receiver)
		lines.push("_Receiver in method form (`x.alias(...)`)_");
	if (arg.sensitive) lines.push("_Sensitive value_");
	return lines.join("\n\n");
}

function namespaceHoverMarkdown(ns: FlowScriptNamespace): string {
	const lines = [`\`\`\`flowscript\nuse ${ns.key}::*\n\`\`\``];
	const members = [...ns.members.keys()].sort();
	const children = [...ns.children.keys()].sort();
	lines.push(
		`_namespace · ${members.length} member${members.length === 1 ? "" : "s"}${
			children.length > 0 ? ` · ${children.length} nested` : ""
		}_`,
	);
	if (members.length > 0)
		lines.push(
			members
				.slice(0, 12)
				.map((member) => `\`${member}\``)
				.join(", ") + (members.length > 12 ? ", …" : ""),
		);
	if (children.length > 0)
		lines.push(
			`Nested: ${children.map((c) => `\`${ns.key}::${c}\``).join(", ")}`,
		);
	return lines.join("\n\n");
}

type MonarchLanguage = Exclude<
	Parameters<Monaco["languages"]["setMonarchTokensProvider"]>[1],
	{ then: unknown }
>;

/** Monarch definition for FlowScript; exported so the tokenizer can be tested without Monaco. */
export const FLOWSCRIPT_MONARCH: MonarchLanguage = {
	defaultToken: "",
	storageKeywords: STORAGE_KEYWORDS,
	controlKeywords: CONTROL_KEYWORDS,
	typeKeywords: TYPE_KEYWORDS,
	constants: CONSTANTS,
	operators: [
		"===",
		"!==",
		"==",
		"!=",
		">=",
		"<=",
		">",
		"<",
		"&&",
		"||",
		"!",
		"+=",
		"-=",
		"*=",
		"/=",
		"+",
		"-",
		"*",
		"/",
		"%",
		"=",
		"?",
		"|",
	],
	symbols: /[=><!~?:&|+\-*/^%]+/,
	escapes: /\\(?:['"\\/bfnrt]|u[0-9A-Fa-f]{4})/,
	tokenizer: {
		root: [
			// Anchor comments carry round-trip identity — highlight distinctly.
			[/\/\/@[a-z]:[^\n]*/, "comment.anchor"],
			[/\/\/.*$/, "comment"],
			// Decorators / annotations (@category, @secret, @readonly, @parallel, …).
			[/@[A-Za-z_][\w]*/, "annotation"],
			// Declaration heads: keyword + declared name.
			// `module` is a keyword only in header position (`module name {`); Monarch matches the
			// rest of the line, not from its start, so the trailing brace is what marks the header.
			[
				/\b(module)\b(\s+)([A-Za-z_$][\w$]*)(?=\s*\{)/,
				["keyword", "white", "entity.name.namespace"],
			],
			// `detached` has no declared name, so the brace alone marks its header.
			[/\bdetached\b(?=\s*\{)/, "keyword"],
			[
				/\b(interface|struct)\b(\s+)([A-Za-z_$][\w$]*)/,
				["keyword", "white", "type.identifier"],
			],
			[
				/\b(function|event)\b(\s+)([A-Za-z_$][\w$]*)/,
				["keyword", "white", "entity.name.function"],
			],
			[
				/\b(const|let)\b(\s+)([A-Za-z_$][\w$]*)/,
				["keyword", "white", "variable"],
			],
			// Namespace path segments (`string::`, `ai::ml::`) and the path separator.
			[/[A-Za-z_$][\w$]*(?=\s*::)/, "entity.name.namespace"],
			[/::/, "delimiter.path"],
			// Method calls (`s.trim()`) before plain property access (`.value`, `.found`).
			[
				/(\.)(\s*)([A-Za-z_$][\w$]*)(?=\s*\()/,
				["delimiter", "white", "entity.name.function"],
			],
			[/(\.)(\s*)([A-Za-z_$][\w$]*)/, ["delimiter", "white", "property"]],
			// Call sites: identifier before `(` (control keywords keep their color).
			[
				/[A-Za-z_$][\w$]*(?=\s*\()/,
				{
					cases: {
						"@controlKeywords": "keyword.control",
						"@storageKeywords": "keyword",
						"@default": "entity.name.function",
					},
				},
			],
			// Named-argument / object keys and type-annotation labels.
			[/[A-Za-z_$][\w$]*(?=\s*:(?!:))/, "variable.parameter"],
			// Bare identifiers, keywords, types, constants.
			[
				/[A-Za-z_$][\w$]*/,
				{
					cases: {
						"@storageKeywords": "keyword",
						"@controlKeywords": "keyword.control",
						"@typeKeywords": "type",
						"@constants": "constant",
						"@default": "identifier",
					},
				},
			],
			{ include: "@whitespace" },
			[/[{}()[\]]/, "@brackets"],
			[/-?\d+\.\d+([eE][+-]?\d+)?/, "number.float"],
			[/-?\d+/, "number"],
			[
				/@symbols/,
				{
					cases: {
						"@operators": "operator",
						"@default": "",
					},
				},
			],
			[/"([^"\\]|\\.)*$/, "string.invalid"],
			[/"/, { token: "string.quote", bracket: "@open", next: "@string" }],
			[/'([^'\\]|\\.)*$/, "string.invalid"],
			[/'/, { token: "string.quote", bracket: "@open", next: "@stringSingle" }],
			[/`/, { token: "string.quote", bracket: "@open", next: "@template" }],
			[/[;,.]/, "delimiter"],
		],
		string: [
			[/[^\\"]+/, "string"],
			[/@escapes/, "string.escape"],
			[/\\./, "string.escape.invalid"],
			[/"/, { token: "string.quote", bracket: "@close", next: "@pop" }],
		],
		stringSingle: [
			[/[^\\']+/, "string"],
			[/@escapes/, "string.escape"],
			[/\\./, "string.escape.invalid"],
			[/'/, { token: "string.quote", bracket: "@close", next: "@pop" }],
		],
		// Template literals: static text is a string, `${ … }` re-enters the expression grammar.
		template: [
			[/\$\{/, { token: "delimiter.template", next: "@templateExpr" }],
			[/[^\\`$]+/, "string"],
			[/@escapes/, "string.escape"],
			[/\\./, "string.escape.invalid"],
			[/\$/, "string"],
			[/`/, { token: "string.quote", bracket: "@close", next: "@pop" }],
		],
		templateExpr: [
			[/\{/, { token: "delimiter.bracket", next: "@templateBlock" }],
			[/\}/, { token: "delimiter.template", next: "@pop" }],
			{ include: "@root" },
		],
		templateBlock: [
			[/\{/, { token: "delimiter.bracket", next: "@templateBlock" }],
			[/\}/, { token: "delimiter.bracket", next: "@pop" }],
			{ include: "@root" },
		],
		whitespace: [[/[ \t\r\n]+/, "white"]],
	},
};

export function registerFlowScriptLanguage(monaco: Monaco): void {
	if (
		monaco.languages
			.getLanguages()
			.some((lang) => lang.id === FLOWSCRIPT_LANGUAGE_ID)
	) {
		return;
	}

	monaco.languages.register({ id: FLOWSCRIPT_LANGUAGE_ID });

	monaco.languages.setLanguageConfiguration(FLOWSCRIPT_LANGUAGE_ID, {
		comments: { lineComment: "//" },
		brackets: [
			["{", "}"],
			["[", "]"],
			["(", ")"],
		],
		autoClosingPairs: [
			{ open: "{", close: "}" },
			{ open: "[", close: "]" },
			{ open: "(", close: ")" },
			{ open: '"', close: '"', notIn: ["string", "comment"] },
			{ open: "'", close: "'", notIn: ["string", "comment"] },
			{ open: "`", close: "`", notIn: ["string", "comment"] },
		],
		surroundingPairs: [
			{ open: "{", close: "}" },
			{ open: "[", close: "]" },
			{ open: "(", close: ")" },
			{ open: '"', close: '"' },
			{ open: "'", close: "'" },
			{ open: "`", close: "`" },
		],
		indentationRules: {
			increaseIndentPattern: /^.*\{[^}"']*$/,
			decreaseIndentPattern: /^\s*\}/,
		},
		folding: {
			markers: {
				start: /^\s*\/\/\s*#?region\b/,
				end: /^\s*\/\/\s*#?endregion\b/,
			},
		},
	});

	monaco.languages.setMonarchTokensProvider(
		FLOWSCRIPT_LANGUAGE_ID,
		FLOWSCRIPT_MONARCH,
	);
}

interface ThemeTokens {
	comment: string;
	anchor: string;
	annotation: string;
	keyword: string;
	control: string;
	type: string;
	typeName: string;
	fn: string;
	variable: string;
	parameter: string;
	property: string;
	string: string;
	number: string;
	constant: string;
	operator: string;
	delimiter: string;
}

const DARK_TOKENS: ThemeTokens = {
	comment: "7d818c",
	anchor: "38bdf8",
	annotation: "c084fc",
	keyword: "60a5fa",
	control: "f472b6",
	type: "a78bfa",
	typeName: "7dd3fc",
	fn: "22d3ee",
	variable: "e5e7eb",
	parameter: "facc15",
	property: "93c5fd",
	string: "86efac",
	number: "fb923c",
	constant: "c084fc",
	operator: "d4d4d8",
	delimiter: "9ca3af",
};

const LIGHT_TOKENS: ThemeTokens = {
	comment: "6b7280",
	anchor: "1d4ed8",
	annotation: "7c3aed",
	keyword: "1d4ed8",
	control: "be185d",
	type: "6d28d9",
	typeName: "0369a1",
	fn: "0e7490",
	variable: "1f2937",
	parameter: "a16207",
	property: "1d4ed8",
	string: "047857",
	number: "b45309",
	constant: "7c3aed",
	operator: "4b5563",
	delimiter: "6b7280",
};

function themeRules(tokens: ThemeTokens) {
	return [
		{ token: "comment", foreground: tokens.comment, fontStyle: "italic" },
		{ token: "comment.anchor", foreground: tokens.anchor, fontStyle: "bold" },
		{ token: "annotation", foreground: tokens.annotation },
		{ token: "tag", foreground: tokens.annotation },
		{ token: "keyword", foreground: tokens.keyword, fontStyle: "bold" },
		{ token: "keyword.control", foreground: tokens.control, fontStyle: "bold" },
		{ token: "type", foreground: tokens.type },
		{
			token: "type.identifier",
			foreground: tokens.typeName,
			fontStyle: "bold",
		},
		{ token: "entity.name.function", foreground: tokens.fn },
		{ token: "entity.name.namespace", foreground: tokens.typeName },
		{ token: "variable", foreground: tokens.variable },
		{ token: "variable.name", foreground: tokens.variable },
		{ token: "variable.parameter", foreground: tokens.parameter },
		{ token: "property", foreground: tokens.property },
		{ token: "identifier", foreground: tokens.variable },
		{ token: "string", foreground: tokens.string },
		{ token: "string.quote", foreground: tokens.string },
		{ token: "string.escape", foreground: tokens.number },
		{ token: "number", foreground: tokens.number },
		{ token: "number.float", foreground: tokens.number },
		{ token: "constant", foreground: tokens.constant },
		{ token: "operator", foreground: tokens.operator },
		{ token: "delimiter", foreground: tokens.delimiter },
		{ token: "delimiter.path", foreground: tokens.delimiter },
		{
			token: "delimiter.template",
			foreground: tokens.annotation,
			fontStyle: "bold",
		},
		// Semantic token types (see the semantic tokens legend in flowscript-language-features).
		{ token: "namespace", foreground: tokens.typeName },
		{ token: "function", foreground: tokens.fn },
		{ token: "method", foreground: tokens.fn },
		{ token: "parameter", foreground: tokens.parameter },
		{ token: "local", foreground: tokens.variable },
		{ token: "event", foreground: tokens.typeName, fontStyle: "bold" },
		{ token: "interface", foreground: tokens.typeName },
	];
}

const DARK_CHROME = {
	"editor.background": "#111116",
	"editor.foreground": "#e5e7eb",
	"editorGutter.background": "#111116",
	"editorLineNumber.foreground": "#686b76",
	"editorLineNumber.activeForeground": "#d4d4d8",
	"editorCursor.foreground": "#f472b6",
	"editor.selectionBackground": "#a855f733",
	"editor.inactiveSelectionBackground": "#a855f71f",
	"editor.lineHighlightBackground": "#ffffff08",
	"editorIndentGuide.background1": "#ffffff12",
	"editorIndentGuide.activeBackground1": "#a855f65c",
	"editorBracketMatch.background": "#a855f61f",
	"editorBracketMatch.border": "#c084fc70",
	"scrollbarSlider.background": "#a1a1aa33",
	"scrollbarSlider.hoverBackground": "#a1a1aa4d",
	"scrollbarSlider.activeBackground": "#a1a1aa66",
};

const LIGHT_CHROME = {
	"editor.background": "#fbfafc",
	"editor.foreground": "#24252b",
	"editorGutter.background": "#fbfafc",
	"editorLineNumber.foreground": "#a6a8b3",
	"editorLineNumber.activeForeground": "#6b7280",
	"editorCursor.foreground": "#ec4899",
	"editor.selectionBackground": "#8b5cf626",
	"editor.inactiveSelectionBackground": "#8b5cf617",
	"editor.lineHighlightBackground": "#11182708",
	"editorIndentGuide.background1": "#11182712",
	"editorIndentGuide.activeBackground1": "#8b5cf64a",
	"editorBracketMatch.background": "#8b5cf61c",
	"editorBracketMatch.border": "#8b5cf670",
	"scrollbarSlider.background": "#71717a33",
	"scrollbarSlider.hoverBackground": "#71717a4d",
	"scrollbarSlider.activeBackground": "#71717a66",
};

export function defineFlowScriptThemes(monaco: Monaco): void {
	monaco.editor.defineTheme(FLOWSCRIPT_THEME_DARK, {
		base: "vs-dark",
		inherit: true,
		rules: themeRules(DARK_TOKENS),
		colors: DARK_CHROME,
	});
	monaco.editor.defineTheme(FLOWSCRIPT_THEME_LIGHT, {
		base: "vs",
		inherit: true,
		rules: themeRules(LIGHT_TOKENS),
		colors: LIGHT_CHROME,
	});
}

/**
 * Register the FlowScript language and apply its theme to one editor instance.
 *
 * Every read-only consumer goes through this: registering the language more than once from
 * separate modules produced mount-order-dependent tokenizer and theme conflicts.
 */
export function setupFlowScriptEditor(monaco: Monaco, isDark: boolean): void {
	registerFlowScriptLanguage(monaco);
	defineFlowScriptThemes(monaco);
	monaco.editor.setTheme(
		isDark ? FLOWSCRIPT_THEME_DARK : FLOWSCRIPT_THEME_LIGHT,
	);
}

export interface Span {
	start: number;
	end: number;
}

export interface MaskedText {
	masked: string;
	/** Offsets of every `${ … }` expression body inside template literals. */
	templateExprs: Span[];
	/** Full spans of every template literal, opening backtick to closing backtick. */
	templates: Span[];
}

type MaskState =
	| { kind: "code"; template: boolean; depth: number; start: number }
	| { kind: "string"; quote: string }
	| { kind: "template"; start: number }
	| { kind: "comment" };

/**
 * Blank out string/comment contents (preserving offsets) so bracket scans stay accurate.
 * Template literal text is blanked too, but `${ … }` bodies stay code (their `${`/`}` fences
 * become spaces) so identifiers inside them still resolve and brackets still balance.
 */
export function maskLiteralsWithSpans(text: string): MaskedText {
	let out = "";
	const templateExprs: Span[] = [];
	const templates: Span[] = [];
	const stack: MaskState[] = [
		{ kind: "code", template: false, depth: 0, start: 0 },
	];
	let i = 0;
	while (i < text.length) {
		const ch = text[i];
		const top = stack[stack.length - 1];
		switch (top.kind) {
			case "code":
				if (ch === '"' || ch === "'") {
					out += ch;
					stack.push({ kind: "string", quote: ch });
				} else if (ch === "`") {
					out += "`";
					stack.push({ kind: "template", start: i });
				} else if (ch === "/" && text[i + 1] === "/") {
					out += "  ";
					i += 2;
					stack.push({ kind: "comment" });
					continue;
				} else if (top.template && ch === "{") {
					top.depth++;
					out += ch;
				} else if (top.template && ch === "}") {
					if (top.depth === 0) {
						stack.pop();
						templateExprs.push({ start: top.start, end: i });
						out += " ";
					} else {
						top.depth--;
						out += ch;
					}
				} else {
					out += ch;
				}
				break;
			case "string":
				if (ch === "\\") {
					out += "  ";
					i += 2;
					continue;
				}
				if (ch === top.quote) {
					out += ch;
					stack.pop();
				} else if (ch === "\n") {
					out += "\n";
					stack.pop();
				} else {
					out += " ";
				}
				break;
			case "template":
				if (ch === "\\") {
					out += "  ";
					i += 2;
					continue;
				}
				if (ch === "`") {
					out += "`";
					stack.pop();
					templates.push({ start: top.start, end: i + 1 });
				} else if (ch === "$" && text[i + 1] === "{") {
					out += "  ";
					i += 2;
					stack.push({ kind: "code", template: true, depth: 0, start: i });
					continue;
				} else {
					out += ch === "\n" ? "\n" : " ";
				}
				break;
			case "comment":
				if (ch === "\n") {
					out += "\n";
					stack.pop();
				} else {
					out += " ";
				}
				break;
		}
		i++;
	}
	for (const state of stack) {
		if (state.kind === "template")
			templates.push({ start: state.start, end: text.length });
	}
	return { masked: out, templateExprs, templates };
}

function maskLiterals(text: string): string {
	return maskLiteralsWithSpans(text).masked;
}

/**
 * Strips the trailing identifier the way `.replace(/[A-Za-z_$][\w$]*$/, "")` does, without
 * regex backtracking (masked literals produce huge space runs that make `$`-anchored
 * patterns quadratic on large documents).
 */
export function stripTrailingWord(s: string): string {
	let start = s.length;
	while (start > 0 && /[\w$]/.test(s[start - 1])) start--;
	if (start === s.length) return s;
	let cut = start;
	while (cut < s.length && /\d/.test(s[cut])) cut++;
	return cut < s.length ? s.slice(0, cut) : s;
}

/** Linear-time replacement for `.replace(/[ \t]*$/, "")` on document-sized strings. */
export function trimTrailingSpacesTabs(s: string): string {
	let end = s.length;
	while (end > 0 && (s[end - 1] === " " || s[end - 1] === "\t")) end--;
	return end === s.length ? s : s.slice(0, end);
}

const IDENT_CHAR = /[\p{L}\p{N}_$]/u;
/** Identifier characters plus `:` so `hash::md5` scans as one call head. */
const PATH_CHAR = /[\p{L}\p{N}_$:]/u;

function inSpan(offset: number, spans: readonly Span[]): boolean {
	return spans.some((span) => offset >= span.start && offset < span.end);
}

/** Index of the bracket closing the one at `open`, or -1 when unbalanced. */
export function matchBracket(text: string, open: number): number {
	let depth = 0;
	for (let i = open; i < text.length; i++) {
		const c = text[i];
		if (c === "(" || c === "[" || c === "{") depth++;
		else if (c === ")" || c === "]" || c === "}") {
			depth--;
			if (depth === 0) return i;
		}
	}
	return -1;
}

export function skipWs(text: string, from: number): number {
	let i = from;
	while (i < text.length && /\s/.test(text[i])) i++;
	return i;
}

/** Splits on top-level commas, keeping each piece's start offset (relative to `base`). */
export function splitTopLevel(
	text: string,
	base = 0,
): { text: string; start: number }[] {
	const pieces: { text: string; start: number }[] = [];
	let depth = 0;
	let start = 0;
	for (let i = 0; i < text.length; i++) {
		const c = text[i];
		if (c === "(" || c === "[" || c === "{") depth++;
		else if (c === ")" || c === "]" || c === "}") depth--;
		else if (c === "," && depth === 0) {
			pieces.push({ text: text.slice(start, i), start: base + start });
			start = i + 1;
		}
	}
	pieces.push({ text: text.slice(start), start: base + start });
	return pieces;
}

/** Brace depth at `offset` in masked text (strings and comments already blanked). */
function braceDepthAt(masked: string, offset: number): number {
	let depth = 0;
	for (let i = 0; i < offset && i < masked.length; i++) {
		if (masked[i] === "{") depth++;
		else if (masked[i] === "}") depth--;
	}
	return depth;
}

export type UseDeclaration = { path: string[]; start: number; end: number } & (
	| { kind: "namespace" }
	| { kind: "glob" }
	| { kind: "members"; members: string[] }
	| { kind: "alias"; alias: string }
	| { kind: "invalid"; error: string }
);

const USE_TREE_RE = new RegExp(
	`^(${IDENT_SRC}(?:\\s*::\\s*${IDENT_SRC})*)(?:\\s*::\\s*(?:(\\*)|\\{([^}]*)\\}))?(?:\\s+as\\s+(${IDENT_SRC}))?$`,
);

function parseUseTree(raw: string, start: number): UseDeclaration | null {
	const leading = raw.length - raw.trimStart().length;
	const tree = raw.trim();
	if (!tree) return null;
	const span = { start: start + leading, end: start + leading + tree.length };
	const m = USE_TREE_RE.exec(tree);
	if (!m) {
		return {
			...span,
			path: [],
			kind: "invalid",
			error: `Malformed use declaration '${tree}'. Expected \`use a::b\`, \`use a::b::*\`, \`use a::{ x, y }\` or \`use a::b as x\`.`,
		};
	}
	const path = m[1].split(PATH_SPLIT_RE);
	if (m[2]) {
		if (m[4])
			return {
				...span,
				path,
				kind: "invalid",
				error: "`as` cannot rename a glob import.",
			};
		return { ...span, path, kind: "glob" };
	}
	if (m[3] !== undefined) {
		if (m[4])
			return {
				...span,
				path,
				kind: "invalid",
				error: "`as` cannot rename a member list.",
			};
		const members = m[3]
			.split(",")
			.map((member) => member.trim())
			.filter(Boolean);
		if (
			members.length === 0 ||
			members.some((member) => !IDENT_RE.test(member))
		)
			return {
				...span,
				path,
				kind: "invalid",
				error: "`use` member list must name at least one identifier.",
			};
		return { ...span, path, kind: "members", members };
	}
	if (m[4]) return { ...span, path, kind: "alias", alias: m[4] };
	return { ...span, path, kind: "namespace" };
}

/**
 * Parses every top-level `use` declaration (Rust use-tree subset): `use a::b` (opens `b`),
 * `use a::b::*` (glob), `use a::{ x, y }` (members), `use a::b as x` (rename), and comma
 * lists of those. Returns one entry per tree with its offsets in `text`; malformed trees come
 * back as `kind: "invalid"` with a message. `use` inside a block is ignored (server-side error).
 */
export function parseUseDeclarations(text: string): UseDeclaration[] {
	const masked = maskLiterals(text);
	const out: UseDeclaration[] = [];
	const stmtRe = /(^|[\n;])[ \t]*use\b/g;
	for (let m = stmtRe.exec(masked); m; m = stmtRe.exec(masked)) {
		const useStart = m.index + m[0].length - 3;
		if (braceDepthAt(masked, useStart) !== 0) continue;
		let tailStart = useStart + 3;
		let end = tailStart;
		let depth = 0;
		while (end < masked.length) {
			const c = masked[end];
			if (c === "{") depth++;
			else if (c === "}") depth--;
			else if (depth <= 0 && (c === "\n" || c === ";")) break;
			end++;
		}
		if (masked[tailStart] === "\n" || masked[tailStart] === ";") continue;
		const tail = masked.slice(tailStart, end);
		for (const piece of splitTopLevel(tail, tailStart)) {
			const decl = parseUseTree(piece.text, piece.start);
			if (decl) out.push(decl);
		}
		tailStart = end;
		stmtRe.lastIndex = Math.max(stmtRe.lastIndex, end);
	}
	return out;
}

export interface UseScope {
	/** Local namespace name → full path (`use ai::ml` → `ml`, `use a::b as x` → `x`). */
	namespaceAliases: Map<string, string[]>;
	/** Bare member name → nodes opened by globs or member lists. */
	openMembers: Map<string, FlowScriptNodeInfo[]>;
	/** Namespace keys referenced by any `use` line (method-dispatch tie-breaker). */
	opened: Set<string>;
}

export function expandPath(path: readonly string[], scope: UseScope): string[] {
	const mapped =
		path.length > 0 ? scope.namespaceAliases.get(path[0]) : undefined;
	return mapped ? [...mapped, ...path.slice(1)] : [...path];
}

function buildUseScope(
	uses: readonly UseDeclaration[],
	index: FlowScriptIndex,
): UseScope {
	const scope: UseScope = {
		namespaceAliases: new Map(),
		openMembers: new Map(),
		opened: new Set(),
	};
	const open = (name: string, info: FlowScriptNodeInfo) => {
		const bucket = scope.openMembers.get(name) ?? [];
		if (!bucket.includes(info)) bucket.push(info);
		scope.openMembers.set(name, bucket);
	};
	for (const use of uses) {
		if (use.kind === "invalid" || use.path.length === 0) continue;
		const path = expandPath(use.path, scope);
		const key = namespaceKey(path);
		scope.opened.add(key);
		const ns = index.namespaces.get(key);
		switch (use.kind) {
			case "namespace":
				scope.namespaceAliases.set(path[path.length - 1], path);
				break;
			case "alias":
				scope.namespaceAliases.set(use.alias, path);
				break;
			case "glob":
				for (const [alias, info] of ns?.members ?? []) open(alias, info);
				break;
			case "members":
				for (const member of use.members) {
					const info = ns?.members.get(member);
					if (info) open(member, info);
				}
				break;
		}
	}
	return scope;
}

/** Line-start offsets of `text` (index i = start of 1-based line i+1). */
function computeLineStartOffsets(text: string): number[] {
	const starts = [0];
	for (let i = 0; i < text.length; i++) {
		if (text[i] === "\n") starts.push(i + 1);
	}
	return starts;
}

function offsetToPosition(
	lineStarts: number[],
	offset: number,
): {
	lineNumber: number;
	column: number;
} {
	let lo = 0;
	let hi = lineStarts.length - 1;
	while (lo < hi) {
		const mid = (lo + hi + 1) >> 1;
		if (lineStarts[mid] <= offset) lo = mid;
		else hi = mid - 1;
	}
	return { lineNumber: lo + 1, column: offset - lineStarts[lo] + 1 };
}

export interface DocumentSymbols {
	variables: Map<string, string | undefined>;
	functions: Set<string>;
	/** `function` name → method class of its first parameter (UFCS), `undefined` = any. */
	functionReceivers: Map<string, string | undefined>;
	interfaces: Set<string>;
	uses: UseDeclaration[];
}

/**
 * Collects callable names declared by top-level and nested event headers.
 *
 * FlowScript supports both the legacy `alias(...) {` spelling and the canonical
 * `nodeType alias(...) {` spelling. In the canonical form the alias is the
 * document symbol; the leading identifier preserves the exact catalog node type.
 */
function collectEventHeaderNames(text: string): Set<string> {
	const names = new Set<string>();
	const eventHeadRe =
		/(?:^|\n)[\t ]*([A-Za-z_$][\w$]*)(?:[\t ]+([A-Za-z_$][\w$]*))?[\t ]*\([^)]*\)[\t ]*\{/g;
	for (let m = eventHeadRe.exec(text); m; m = eventHeadRe.exec(text)) {
		if (!KEYWORD_SET.has(m[1])) names.add(m[2] ?? m[1]);
	}
	return names;
}

const DESTRUCTURE_SRC = "\\{([^}]*)\\}";
const LOOP_BINDING_SRC = `(?:(${IDENT_SRC})|\\[\\s*(${IDENT_SRC})\\s*,\\s*(${IDENT_SRC})\\s*\\])`;

/** Parses `a, b: c` destructuring members into `[localName, sourceField]` pairs. */
function destructureMembers(body: string): [string, string][] {
	const out: [string, string][] = [];
	for (const part of body.split(",")) {
		const m = new RegExp(
			`^\\s*(${IDENT_SRC})\\s*(?::\\s*(${IDENT_SRC}))?\\s*$`,
		).exec(part);
		if (m) out.push([m[2] ?? m[1], m[1]]);
	}
	return out;
}

/** Scans the FlowScript document for its own declared variables, functions and interfaces. */
function collectDocumentSymbols(masked: string): DocumentSymbols {
	const variables = new Map<string, string | undefined>();
	const functions = new Set<string>();
	const functionReceivers = new Map<string, string | undefined>();
	const interfaces = new Set<string>();

	const declRe = new RegExp(
		`(?:^|[\\n;{}])\\s*(?:const|let)\\s+(?:(${IDENT_SRC})\\s*(?::\\s*([^=\\n]+?))?|${DESTRUCTURE_SRC})\\s*=`,
		"g",
	);
	for (let m = declRe.exec(masked); m; m = declRe.exec(masked)) {
		if (m[1]) variables.set(m[1], m[2]?.trim());
		else if (m[3] !== undefined)
			for (const [local] of destructureMembers(m[3]))
				variables.set(local, undefined);
	}
	const loopRe = new RegExp(
		`for\\s*\\(\\s*(?:const|let)\\s+${LOOP_BINDING_SRC}\\s+(?:of|in)\\b`,
		"g",
	);
	for (let m = loopRe.exec(masked); m; m = loopRe.exec(masked)) {
		for (const name of [m[1], m[2], m[3]]) {
			if (name && !variables.has(name)) variables.set(name, undefined);
		}
	}
	const fnRe = new RegExp(
		`\\b(function|event)\\s+(${IDENT_SRC})\\s*\\(\\s*(?:(${IDENT_SRC})\\s*:\\s*([^,)]+))?`,
		"g",
	);
	for (let m = fnRe.exec(masked); m; m = fnRe.exec(masked)) {
		functions.add(m[2]);
		if (m[1] === "function")
			functionReceivers.set(
				m[2],
				m[4] ? classOfValue(parseTypeAnnotation(m[4])) : undefined,
			);
	}
	for (const name of collectEventHeaderNames(masked)) functions.add(name);
	const ifaceRe = /\b(?:interface|struct)\s+([A-Za-z_$][\w$]*)/g;
	for (let m = ifaceRe.exec(masked); m; m = ifaceRe.exec(masked))
		interfaces.add(m[1]);

	return {
		variables,
		functions,
		functionReceivers,
		interfaces,
		uses: parseUseDeclarations(masked),
	};
}

interface StructMember {
	name: string;
	typeString: string;
	description: string;
	/** Sub-schema (carrying $defs) for nested navigation, when this member is itself a struct. */
	schema?: string;
}

/** A value whose members can be navigated: a multi-output node result, or a struct schema. */
type MemberSource =
	| { kind: "node"; info: FlowScriptNodeInfo }
	| { kind: "struct"; title?: string; members: StructMember[] };

type Schema = Record<string, unknown>;

const schemaParseCache = new Map<string, Schema | null>();
const SCHEMA_CACHE_LIMIT = 1000;

function parseSchema(str: string): Schema | null {
	const cached = schemaParseCache.get(str);
	if (cached !== undefined) return cached;
	let parsed: Schema | null = null;
	try {
		const value = JSON.parse(str);
		parsed = value && typeof value === "object" ? (value as Schema) : null;
	} catch {
		parsed = null;
	}
	// Bound the cache so long editing sessions with churning inline schemas
	// don't leak unbounded memory; evict the oldest (Map preserves insertion order).
	if (schemaParseCache.size >= SCHEMA_CACHE_LIMIT) {
		const oldest = schemaParseCache.keys().next().value;
		if (oldest !== undefined) schemaParseCache.delete(oldest);
	}
	schemaParseCache.set(str, parsed);
	return parsed;
}

function resolveRef(schema: unknown, defs: Schema): Schema | undefined {
	let s = schema as Schema | undefined;
	let guard = 0;
	while (s && typeof s.$ref === "string" && guard++ < 12) {
		const m = /^#\/(?:\$defs|definitions)\/(.+)$/.exec(s.$ref as string);
		if (!m) break;
		s = defs[m[1]] as Schema | undefined;
	}
	return s;
}

/** Picks the object-shaped branch of a (possibly $ref / anyOf-with-null) schema. */
function objectBranch(schema: unknown, defs: Schema): Schema | undefined {
	const s = resolveRef(schema, defs);
	if (!s) return undefined;
	const union = (s.anyOf ?? s.oneOf) as unknown[] | undefined;
	if (Array.isArray(union)) {
		for (const branch of union) {
			const rb = resolveRef(branch, defs);
			if (rb?.properties) return rb;
		}
		return undefined;
	}
	return s.properties ? s : undefined;
}

function schemaTypeLabel(schema: unknown, defs: Schema): string {
	const s = resolveRef(schema, defs);
	if (!s) return "any";
	if (typeof s.title === "string") return s.title;
	const union = (s.anyOf ?? s.oneOf) as unknown[] | undefined;
	if (Array.isArray(union)) {
		const parts = union
			.map((b) => schemaTypeLabel(b, defs))
			.filter((t) => t !== "null" && t !== "any");
		return parts.length ? [...new Set(parts)].join(" | ") : "any";
	}
	if (s.type === "array") return `${schemaTypeLabel(s.items, defs)}[]`;
	const type = Array.isArray(s.type)
		? (s.type as string[]).find((x) => x !== "null")
		: s.type;
	switch (type) {
		case "integer":
			return "int";
		case "number":
			return "float";
		case "boolean":
			return "bool";
		case "string":
			return "string";
		case "object":
			return "object";
		default:
			return "any";
	}
}

/** Extracts a struct's members (schema properties) from a struct pin's JSON-schema string. */
function structFromSchema(
	schemaStr?: string,
): { title?: string; members: StructMember[] } | null {
	if (!schemaStr) return null;
	const root = parseSchema(schemaStr);
	if (!root) return null;
	const defs = ((root.$defs ?? root.definitions) as Schema) ?? {};
	const obj = objectBranch(root, defs);
	const properties = obj?.properties as Schema | undefined;
	if (!properties) return null;
	const required = new Set<string>(
		Array.isArray(obj?.required) ? (obj.required as string[]) : [],
	);
	const members: StructMember[] = Object.entries(properties).map(
		([name, prop]) => {
			const branch = objectBranch(prop, defs);
			return {
				name,
				typeString:
					schemaTypeLabel(prop, defs) + (required.has(name) ? "" : "?"),
				description: (resolveRef(prop, defs)?.description as string) ?? "",
				schema: branch ? JSON.stringify({ ...branch, $defs: defs }) : undefined,
			};
		},
	);
	return {
		title: (root.title as string) ?? (obj?.title as string | undefined),
		members,
	};
}

function structSource(
	schema?: string,
	fallbackTitle?: string,
): MemberSource | null {
	const s = structFromSchema(schema);
	return s
		? { kind: "struct", title: s.title ?? fallbackTitle, members: s.members }
		: null;
}

function outputToSource(output: FlowScriptOutput): MemberSource | null {
	if (output.dataType !== IVariableType.Struct || !output.schema) return null;
	return structSource(output.schema, output.schemaTitle);
}

function sourceMembers(
	source: MemberSource,
): { name: string; typeString: string; description: string }[] {
	if (source.kind === "node") {
		return source.info.outputs.map((o) => ({
			name: o.name,
			typeString: o.typeString,
			description: o.description,
		}));
	}
	return source.members.map((m) => ({
		name: m.name,
		typeString: m.typeString,
		description: m.description,
	}));
}

type TypeGroup =
	| "string"
	| "number"
	| "bool"
	| "struct"
	| "date"
	| "path"
	| "bytes"
	| "any"
	| "null";

export interface ValueType {
	group: TypeGroup;
	isArray: boolean;
	schemaTitle?: string;
	dataType?: IVariableType;
	container?: IValueType;
	/** Set when the value is a bare call to a multi-output node (a result bundle, not a value). */
	multiOutput?: { node: string; outputs: string[] };
}

function groupOf(dataType: IVariableType): TypeGroup {
	switch (dataType) {
		case IVariableType.String:
			return "string";
		case IVariableType.Integer:
		case IVariableType.Float:
			return "number";
		case IVariableType.Boolean:
			return "bool";
		case IVariableType.Struct:
			return "struct";
		case IVariableType.Date:
			return "date";
		case IVariableType.PathBuf:
			return "path";
		case IVariableType.Byte:
			return "bytes";
		default:
			return "any";
	}
}

const pinValueType = (pin: {
	dataType: IVariableType;
	container: IValueType;
	schemaTitle?: string;
}): ValueType => ({
	group: groupOf(pin.dataType),
	isArray: pin.container === IValueType.Array,
	schemaTitle: pin.schemaTitle,
	dataType: pin.dataType,
	container: pin.container,
});

/** Method class of a value (`x.` completion and dispatch), `undefined` when unknown. */
function classOfValue(value: ValueType | null): string | undefined {
	if (!value || value.multiOutput) return undefined;
	if (value.container === IValueType.HashMap) return "map";
	if (value.container === IValueType.HashSet) return "set";
	if (value.isArray) return "array";
	if (value.dataType)
		return methodClassFor(value.dataType, IValueType.Normal, value.schemaTitle);
	switch (value.group) {
		case "string":
			return "string";
		case "bool":
			return "bool";
		case "struct":
			return value.schemaTitle ?? "struct";
		case "date":
			return "datetime";
		case "path":
			return "path";
		case "bytes":
			return "bytes";
		default:
			return undefined;
	}
}

function parseTypeAnnotation(text: string): ValueType {
	let base = text.trim();
	let isArray = false;
	let container: IValueType | undefined;
	if (base.endsWith("[]")) {
		isArray = true;
		container = IValueType.Array;
		base = base.slice(0, -2).trim();
	}
	const setMatch = /^Set<(.+)>$/.exec(base);
	if (setMatch) {
		isArray = true;
		container = IValueType.HashSet;
		base = setMatch[1].trim();
	}
	if (/^Map</.test(base))
		return { group: "any", isArray: false, container: IValueType.HashMap };
	if (base.includes("|") || base.includes("("))
		return { group: "any", isArray, container };
	switch (base.toLowerCase()) {
		case "string":
			return {
				group: "string",
				isArray,
				container,
				dataType: IVariableType.String,
			};
		case "int":
			return {
				group: "number",
				isArray,
				container,
				dataType: IVariableType.Integer,
			};
		case "float":
			return {
				group: "number",
				isArray,
				container,
				dataType: IVariableType.Float,
			};
		case "number":
			return { group: "number", isArray, container };
		case "bool":
		case "boolean":
			return {
				group: "bool",
				isArray,
				container,
				dataType: IVariableType.Boolean,
			};
		case "date":
			return {
				group: "date",
				isArray,
				container,
				dataType: IVariableType.Date,
			};
		case "path":
		case "pathbuf":
			return {
				group: "path",
				isArray,
				container,
				dataType: IVariableType.PathBuf,
			};
		case "byte":
		case "bytes":
			return {
				group: "bytes",
				isArray,
				container,
				dataType: IVariableType.Byte,
			};
		case "struct":
		case "object":
			return {
				group: "struct",
				isArray,
				container,
				dataType: IVariableType.Struct,
			};
		case "any":
		case "generic":
		case "void":
			return { group: "any", isArray, container };
	}
	if (/^[A-Z]/.test(base))
		return {
			group: "struct",
			isArray,
			container,
			schemaTitle: base,
			dataType: IVariableType.Struct,
		};
	return { group: "any", isArray, container };
}

function memberValueType(member: StructMember): ValueType {
	return parseTypeAnnotation(member.typeString.replace(/\?$/, ""));
}

/** Maps document variables to a resolved type from annotations or literal initializers. */
function collectVariableTypes(masked: string): Map<string, ValueType> {
	const map = new Map<string, ValueType>();
	const annotated =
		/(?:^|[\n;{}])\s*(?:const|let)\s+([A-Za-z_$][\w$]*)\s*:\s*([^=\n]+?)\s*=/g;
	for (let m = annotated.exec(masked); m; m = annotated.exec(masked)) {
		map.set(m[1], parseTypeAnnotation(m[2]));
	}
	const literal =
		/(?:^|[\n;{}])\s*(?:const|let)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:("|'|`|\[|\{)|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|\b(true|false)\b)/g;
	for (let m = literal.exec(masked); m; m = literal.exec(masked)) {
		if (map.has(m[1])) continue;
		if (m[2] === '"' || m[2] === "'" || m[2] === "`")
			map.set(m[1], {
				group: "string",
				isArray: false,
				dataType: IVariableType.String,
			});
		else if (m[2] === "[")
			map.set(m[1], {
				group: "any",
				isArray: true,
				container: IValueType.Array,
			});
		else if (m[2] === "{")
			map.set(m[1], {
				group: "struct",
				isArray: false,
				dataType: IVariableType.Struct,
			});
		else if (m[3])
			map.set(m[1], {
				group: "number",
				isArray: false,
				dataType: /[.eE]/.test(m[3])
					? IVariableType.Float
					: IVariableType.Integer,
			});
		else
			map.set(m[1], {
				group: "bool",
				isArray: false,
				dataType: IVariableType.Boolean,
			});
	}
	return map;
}

export interface VariableExprs {
	/** `const x = <rhs>` / `const { a, b: c } = <rhs>` → expression text per local name. */
	exprs: Map<string, string>;
	/** `for (const x of <rhs>)` → the iterated expression. */
	loops: Map<string, string>;
	/** `for (const [i, x] of …)` index bindings. */
	indexVars: Set<string>;
}

/** Maps each binding to its RHS expression text (first declaration wins). */
function collectVariableExprs(masked: string): VariableExprs {
	const exprs = new Map<string, string>();
	const loops = new Map<string, string>();
	const indexVars = new Set<string>();
	const scanRhs = (from: number): string => {
		let i = from;
		let depth = 0;
		while (i < masked.length) {
			const c = masked[i];
			if (c === "{" || c === "[" || c === "(") depth++;
			else if (c === "}" || c === "]" || c === ")") {
				if (depth === 0) break;
				depth--;
			} else if (depth === 0 && (c === "\n" || c === ";" || c === ",")) break;
			i++;
		}
		return masked.slice(from, i).trim();
	};
	const declRe = new RegExp(
		`(?:^|[\\n;{}])[ \\t]*(?:const|let)\\s+(?:(${IDENT_SRC})\\s*(?::[^=\\n]+)?|${DESTRUCTURE_SRC})\\s*=[ \\t]*`,
		"g",
	);
	for (let m = declRe.exec(masked); m; m = declRe.exec(masked)) {
		const rhs = scanRhs(declRe.lastIndex);
		if (m[1]) {
			if (!exprs.has(m[1])) exprs.set(m[1], rhs);
		} else if (m[2] !== undefined) {
			for (const [local, field] of destructureMembers(m[2])) {
				if (!exprs.has(local)) exprs.set(local, `(${rhs}).${field}`);
			}
		}
	}
	const loopRe = new RegExp(
		`for\\s*\\(\\s*(?:const|let)\\s+${LOOP_BINDING_SRC}\\s+of[ \\t]+`,
		"g",
	);
	for (let m = loopRe.exec(masked); m; m = loopRe.exec(masked)) {
		const rhs = scanRhs(loopRe.lastIndex);
		const element = m[1] ?? m[3];
		if (element && !loops.has(element)) loops.set(element, rhs);
		if (m[2]) indexVars.add(m[2]);
	}
	return { exprs, loops, indexVars };
}

export interface TypeEnv {
	index: FlowScriptIndex;
	scope: UseScope;
	symbols: DocumentSymbols;
	docVars: Map<string, ValueType>;
	vars: VariableExprs;
}

export function buildTypeEnv(masked: string, index: FlowScriptIndex): TypeEnv {
	const symbols = collectDocumentSymbols(masked);
	return {
		index,
		scope: buildUseScope(symbols.uses, index),
		symbols,
		docVars: collectVariableTypes(masked),
		vars: collectVariableExprs(masked),
	};
}

/** Rebuilds the `use`-scope of a type environment against a (possibly different) index. */
export function buildUseScopeFor(
	uses: readonly UseDeclaration[],
	index: FlowScriptIndex,
): UseScope {
	return buildUseScope(uses, index);
}

/**
 * The masked text and type environment of one document version, shared by completion,
 * hover, signature help, diagnostics and the structural analysis so a change burst pays
 * for masking and environment collection once instead of once per provider.
 */
export interface FlowScriptEnvDoc {
	text: string;
	masked: string;
	templateExprs: Span[];
	templates: Span[];
	env: TypeEnv;
}

const ENV_DOC_CACHE_LIMIT = 4;
const envDocCache = new Map<string, FlowScriptEnvDoc>();

export function getFlowScriptEnvDoc(
	text: string,
	index: FlowScriptIndex,
): FlowScriptEnvDoc {
	const cached = envDocCache.get(text);
	if (cached && cached.env.index === index) return cached;
	const { masked, templateExprs, templates } = maskLiteralsWithSpans(text);
	const envDoc: FlowScriptEnvDoc = {
		text,
		masked,
		templateExprs,
		templates,
		env: buildTypeEnv(masked, index),
	};
	if (envDocCache.size >= ENV_DOC_CACHE_LIMIT) {
		const oldest = envDocCache.keys().next().value;
		if (oldest !== undefined) envDocCache.delete(oldest);
	}
	envDocCache.set(text, envDoc);
	return envDoc;
}

function isBoundName(name: string, env: TypeEnv): boolean {
	return (
		env.docVars.has(name) ||
		env.vars.exprs.has(name) ||
		env.vars.loops.has(name) ||
		env.vars.indexVars.has(name) ||
		env.symbols.functions.has(name)
	);
}

export interface ExprInfo {
	value: ValueType | null;
	source: MemberSource | null;
	/** The node whose result bundle this expression is, so its outputs stay addressable. */
	node?: FlowScriptNodeInfo;
	/** Set when the expression names a namespace (`string`, `ai.ml`) rather than a value. */
	namespace?: string[];
}

const UNKNOWN: ExprInfo = { value: null, source: null };
const scalar = (group: TypeGroup, dataType?: IVariableType): ExprInfo => ({
	value: { group, isArray: false, dataType },
	source: null,
});

function callResult(info: FlowScriptNodeInfo): ExprInfo {
	const value: ValueType | null = info.defaultOutput
		? pinValueType(info.defaultOutput)
		: info.outputs.length > 1
			? {
					group: "struct",
					isArray: false,
					multiOutput: {
						node: displayName(info),
						outputs: info.outputs.map((o) => o.name),
					},
				}
			: null;
	const source: MemberSource | null =
		info.outputs.length === 1
			? outputToSource(info.outputs[0])
			: info.outputs.length > 1
				? { kind: "node", info }
				: null;
	return { value, source, node: info };
}

function memberOf(current: ExprInfo, member: string): ExprInfo {
	if (current.node) {
		const out = current.node.outputs.find((o) => o.name === member);
		if (out) return { value: pinValueType(out), source: outputToSource(out) };
	}
	if (current.source?.kind === "struct") {
		const found = current.source.members.find((m) => m.name === member);
		if (found)
			return {
				value: memberValueType(found),
				source: found.schema ? structSource(found.schema) : null,
			};
	}
	if (member === "length" && current.value?.isArray)
		return scalar("number", IVariableType.Integer);
	return UNKNOWN;
}

function elementOf(current: ExprInfo): ExprInfo {
	const v = current.value;
	if (!v?.isArray) return UNKNOWN;
	return {
		value: {
			group: v.group,
			isArray: false,
			schemaTitle: v.schemaTitle,
			dataType: v.dataType,
		},
		source: current.source,
	};
}

export interface CallResolution {
	info?: FlowScriptNodeInfo;
	candidates: FlowScriptNodeInfo[];
	/** The name is a user-declared function (UFCS in method form). */
	userFunction: boolean;
	/** `method` binds the receiver pin; `static` (flat, qualified or namespace walk) does not. */
	form: "static" | "method";
	/** Receiver class for method calls when it could be determined. */
	receiverClass?: string;
	/** Path calls: the expanded namespace exists in the catalog. */
	namespaceKnown?: boolean;
	path?: string[];
}

/** The receiver pin is bound only when the call resolved in method form. */
export function receiverIsBound(resolution: CallResolution): boolean {
	return (
		resolution.form === "method" && resolution.info?.receiver !== undefined
	);
}

/**
 * Narrows ambiguous candidates by argument shape (every named key must be an input of the
 * node, the receiver excluded in method form), mirroring reconcile's arg-shape filter.
 */
function narrowByArgShape(
	resolution: CallResolution,
	argNames: readonly string[],
): CallResolution {
	if (resolution.candidates.length < 2 || argNames.length === 0)
		return resolution;
	const fitting = resolution.candidates.filter((candidate) => {
		const inputs = new Set(
			(resolution.form === "method"
				? methodParams(candidate)
				: candidate.args
			).map((arg) => arg.name),
		);
		return argNames.every((name) => inputs.has(name));
	});
	if (fitting.length === 0) return resolution;
	return { ...resolution, info: fitting[0], candidates: fitting };
}

function resolvePathCall(
	path: readonly string[],
	member: string,
	env: TypeEnv,
): CallResolution {
	const full = expandPath(path, env.scope);
	const ns = env.index.namespaces.get(namespaceKey(full));
	const info = ns?.members.get(member);
	return {
		info,
		candidates: info ? [info] : [],
		userFunction: false,
		form: "static",
		namespaceKnown: ns !== undefined,
		path: full,
	};
}

function resolveBareCall(name: string, env: TypeEnv): CallResolution {
	if (env.symbols.functions.has(name))
		return { candidates: [], userFunction: true, form: "static" };
	const flat = env.index.byName.get(name);
	const opened = env.scope.openMembers.get(name) ?? [];
	const candidates = flat
		? [flat, ...opened.filter((c) => c !== flat)]
		: [...opened];
	return {
		info: candidates[0],
		candidates,
		userFunction: false,
		form: "static",
	};
}

function isTitledStruct(cls: string): boolean {
	return !VALUE_CLASSES.has(cls) && cls !== UNIVERSAL_CLASS;
}

function methodCandidates(
	cls: string | undefined,
	member: string,
	index: FlowScriptIndex,
): FlowScriptNodeInfo[] {
	const bucket = (c: string) => index.methods.get(c)?.get(member) ?? [];
	const out: FlowScriptNodeInfo[] = [];
	const push = (infos: FlowScriptNodeInfo[]) => {
		for (const info of infos) if (!out.includes(info)) out.push(info);
	};
	if (cls) {
		push(bucket(cls));
		if (isTitledStruct(cls)) push(bucket("struct"));
		push(bucket(UNIVERSAL_CLASS));
		return out;
	}
	for (const table of index.methods.values()) push(table.get(member) ?? []);
	return out;
}

function preferOpened(
	candidates: FlowScriptNodeInfo[],
	scope: UseScope,
): FlowScriptNodeInfo[] {
	if (candidates.length < 2 || scope.opened.size === 0) return candidates;
	const opened = candidates.filter((c) =>
		c.namespace ? scope.opened.has(namespaceKey(c.namespace)) : false,
	);
	return opened.length > 0 ? opened : candidates;
}

function resolveMethod(
	receiver: ExprInfo,
	member: string,
	env: TypeEnv,
): CallResolution {
	const receiverClass = classOfValue(receiver.value);
	if (env.symbols.functions.has(member))
		return {
			candidates: [],
			userFunction: true,
			form: "method",
			receiverClass,
		};
	const candidates = preferOpened(
		methodCandidates(receiverClass, member, env.index),
		env.scope,
	);
	return {
		info: candidates[0],
		candidates,
		userFunction: false,
		form: "method",
		receiverClass,
	};
}

function resolveVariable(name: string, env: TypeEnv, depth: number): ExprInfo {
	const annotated = env.docVars.get(name);
	if (annotated) return { value: annotated, source: null };
	const rhs = env.vars.exprs.get(name);
	if (rhs !== undefined) {
		const resolved = evaluateExpr(rhs, env, depth + 1);
		if (resolved.value || resolved.source || resolved.node) return resolved;
	}
	const loopRhs = env.vars.loops.get(name);
	if (loopRhs !== undefined) {
		const iterated = evaluateExpr(loopRhs, env, depth + 1);
		// Legacy loop handles (`for (const it of controlForEach(...))`) keep the node bundle.
		if (iterated.node?.outputs.some((o) => o.rawName === "value"))
			return iterated;
		return elementOf(iterated);
	}
	if (env.vars.indexVars.has(name))
		return scalar("number", IVariableType.Integer);
	return UNKNOWN;
}

const EXPR_CACHE_LIMIT = 4000;
const exprCaches = new WeakMap<TypeEnv, Map<string, ExprInfo>>();

/**
 * Resolves the type of a FlowScript expression: literals, variables (following their bindings),
 * flat / qualified / method calls (through the node's default output), member chains over
 * node outputs and JSON-schema struct fields, indexing, and bare namespace references.
 *
 * Top-level results are memoized per environment: providers and the linter evaluate the
 * same argument/receiver expressions many times per document version.
 */
export function evaluateExpr(expr: string, env: TypeEnv, depth = 0): ExprInfo {
	if (depth > 8) return UNKNOWN;
	if (depth === 0) {
		let cache = exprCaches.get(env);
		if (!cache) {
			cache = new Map();
			exprCaches.set(env, cache);
		}
		const cached = cache.get(expr);
		if (cached) return cached;
		const result = evaluateExprUncached(expr, env, 0);
		if (cache.size >= EXPR_CACHE_LIMIT) cache.clear();
		cache.set(expr, result);
		return result;
	}
	return evaluateExprUncached(expr, env, depth);
}

function evaluateExprUncached(
	expr: string,
	env: TypeEnv,
	depth: number,
): ExprInfo {
	const e = expr.trim();
	if (!e) return UNKNOWN;
	if (e === "null") return scalar("null");
	if (e === "true" || e === "false")
		return scalar("bool", IVariableType.Boolean);
	if (e[0] === "!") return scalar("bool", IVariableType.Boolean);
	if (e[0] === "-" && !/^-\s*\d/.test(e)) {
		const operand = evaluateExpr(e.slice(1), env, depth + 1);
		return operand.value?.group === "number" ? operand : scalar("number");
	}
	if (e[0] === "[")
		return {
			value: { group: "any", isArray: true, container: IValueType.Array },
			source: null,
		};
	if (e[0] === "{") return scalar("struct", IVariableType.Struct);

	let pos = 0;
	let current: ExprInfo;
	if (e[0] === "(") {
		const close = matchBracket(e, 0);
		if (close < 0) return UNKNOWN;
		current = evaluateExpr(e.slice(1, close), env, depth + 1);
		pos = close + 1;
	} else if (e[0] === '"' || e[0] === "'" || e[0] === "`") {
		const close = e.indexOf(e[0], 1);
		current = scalar("string", IVariableType.String);
		if (close < 0) return current;
		pos = close + 1;
	} else if (/^-?\d/.test(e)) {
		const num = /^-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/.exec(e);
		if (!num) return UNKNOWN;
		current = scalar(
			"number",
			/[.eE]/.test(num[0]) ? IVariableType.Float : IVariableType.Integer,
		);
		pos = num[0].length;
	} else {
		const head = HEAD_RE.exec(e);
		if (!head) return UNKNOWN;
		pos = head[0].length;
		const segments = head[0].split(PATH_SPLIT_RE);
		const after = skipWs(e, pos);
		if (e[after] === "(") {
			const close = matchBracket(e, after);
			if (close < 0) return UNKNOWN;
			const resolution =
				segments.length > 1
					? resolvePathCall(
							segments.slice(0, -1),
							segments[segments.length - 1],
							env,
						)
					: resolveBareCall(segments[0], env);
			current = resolution.info ? callResult(resolution.info) : UNKNOWN;
			pos = close + 1;
		} else if (segments.length > 1) {
			return UNKNOWN;
		} else {
			const name = segments[0];
			current = resolveVariable(name, env, depth);
			if (current === UNKNOWN && !isBoundName(name, env)) {
				const path = expandPath([name], env.scope);
				if (env.index.namespaces.has(namespaceKey(path)))
					current = { value: null, source: null, namespace: path };
			}
		}
	}

	for (;;) {
		pos = skipWs(e, pos);
		if (pos >= e.length) break;
		const ch = e[pos];
		if (ch === ".") {
			const m = /^\.\s*([A-Za-z_$][\w$]*)/.exec(e.slice(pos));
			if (!m) return UNKNOWN;
			const member = m[1];
			pos += m[0].length;
			const after = skipWs(e, pos);
			if (e[after] === "(") {
				const close = matchBracket(e, after);
				if (close < 0) return UNKNOWN;
				const resolution = current.namespace
					? resolvePathCall(current.namespace, member, env)
					: resolveMethod(current, member, env);
				current = resolution.info ? callResult(resolution.info) : UNKNOWN;
				pos = close + 1;
			} else if (current.namespace) {
				const child = env.index.namespaces.get(
					namespaceKey([...current.namespace, member]),
				);
				current = child
					? { value: null, source: null, namespace: child.path }
					: UNKNOWN;
			} else {
				current = memberOf(current, member);
			}
		} else if (ch === "[") {
			const close = matchBracket(e, pos);
			if (close < 0) return UNKNOWN;
			current = elementOf(current);
			pos = close + 1;
		} else {
			return UNKNOWN;
		}
	}
	return current;
}

/** Extracts the trailing primary expression (literal / identifier / call / member chain). */
export function extractTrailingExpr(s: string): string {
	let i = s.length;
	let depth = 0;
	while (i > 0) {
		const c = s[i - 1];
		if (c === ")" || c === "]" || c === "}") {
			depth++;
			i--;
		} else if (c === "(" || c === "[" || c === "{") {
			if (depth === 0) break;
			depth--;
			i--;
		} else if (depth > 0) {
			i--;
		} else if (c === '"' || c === "'" || c === "`") {
			const open = s.lastIndexOf(c, i - 2);
			i = open < 0 ? i - 1 : open;
			break;
		} else if (IDENT_CHAR.test(c) || c === "." || c === ":") {
			i--;
		} else {
			break;
		}
	}
	return s.slice(i);
}

export type CallHead =
	| { kind: "path"; path: string[]; member: string; display: string }
	| { kind: "method"; receiverExpr: string; member: string; display: string }
	| { kind: "bare"; member: string; display: string };

/** Reads the callee spelled before `(` at `parenIndex`: `a::b::c`, `expr.method` or `name`. */
export function callHeadBefore(
	text: string,
	parenIndex: number,
): CallHead | undefined {
	let j = parenIndex - 1;
	while (j >= 0 && /\s/.test(text[j])) j--;
	const end = j + 1;
	while (j >= 0 && PATH_CHAR.test(text[j])) j--;
	const headText = text.slice(j + 1, end);
	if (!headText || headText.replace(/::/g, "").includes(":")) return undefined;
	const segments = headText.split(PATH_SPLIT_RE);
	if (segments.some((segment) => !IDENT_RE.test(segment))) return undefined;
	const before = text.slice(0, j + 1).trimEnd();
	if (before.endsWith("@")) return undefined;
	const member = segments[segments.length - 1];
	if (segments.length > 1)
		return {
			kind: "path",
			path: segments.slice(0, -1),
			member,
			display: segments.join("::"),
		};
	if (before.endsWith(".")) {
		const receiverExpr = extractTrailingExpr(before.slice(0, -1));
		return {
			kind: "method",
			receiverExpr,
			member,
			display: `${receiverExpr}.${member}`,
		};
	}
	if (KEYWORD_SET.has(member)) return undefined;
	return { kind: "bare", member, display: member };
}

export function resolveCallHead(
	head: CallHead,
	env: TypeEnv,
	argNames: readonly string[] = [],
): CallResolution {
	const resolved = (() => {
		switch (head.kind) {
			case "path":
				return resolvePathCall(head.path, head.member, env);
			case "bare":
				return resolveBareCall(head.member, env);
			case "method": {
				const receiver = evaluateExpr(head.receiverExpr, env);
				if (receiver.namespace)
					return resolvePathCall(receiver.namespace, head.member, env);
				return resolveMethod(receiver, head.member, env);
			}
		}
	})();
	return narrowByArgShape(resolved, argNames);
}

export interface CallContext {
	callName: string;
	info?: FlowScriptNodeInfo;
	candidates: FlowScriptNodeInfo[];
	/** Method form: the receiver pin is already bound and must not be named again. */
	receiverBound: boolean;
	/** Positional arguments written before the named-args object (or the active index). */
	positionalCount: number;
	/** Data inputs still open for binding by name. */
	params: FlowScriptArg[];
	existingKeys: string[];
	mode: "key" | "value" | "positional";
	activeArg?: string;
}

/**
 * Given masked text up to the cursor, determine whether the cursor sits inside a call's
 * argument list, which call it is (flat, qualified or method spelling), the keys already
 * present, and whether we are typing a positional argument, a key or a value.
 */
export function analyzeContext(
	maskedBefore: string,
	env: TypeEnv,
): CallContext | null {
	const stack: { ch: string; head?: CallHead; open: number }[] = [];
	for (let i = 0; i < maskedBefore.length; i++) {
		const ch = maskedBefore[i];
		if (ch === "(" || ch === "{" || ch === "[") {
			stack.push({
				ch,
				head: ch === "(" ? callHeadBefore(maskedBefore, i) : undefined,
				open: i,
			});
		} else if (ch === ")" || ch === "}" || ch === "]") {
			stack.pop();
		}
	}

	const top = stack[stack.length - 1];
	if (!top) return null;
	let call: { head: CallHead; open: number };
	let braceOpen: number | undefined;
	if (top.ch === "{") {
		const parent = stack[stack.length - 2];
		if (!parent || parent.ch !== "(" || !parent.head) return null;
		call = { head: parent.head, open: parent.open };
		braceOpen = top.open;
	} else if (top.ch === "(" && top.head) {
		call = { head: top.head, open: top.open };
	} else {
		return null;
	}

	const argText = maskedBefore.slice(call.open + 1, braceOpen ?? undefined);
	const pieces = splitTopLevel(argText);

	// Keys already written inside the named-args object; `segment` is the arg under the cursor.
	const existingKeys: string[] = [];
	let activeKey: string | undefined;
	if (braceOpen !== undefined) {
		const body = maskedBefore.slice(braceOpen + 1);
		let depth = 0;
		let segment = "";
		const flush = () => {
			const match = /^\s*([A-Za-z_$][\w$]*)\s*:/.exec(segment);
			if (match) existingKeys.push(match[1]);
		};
		for (const ch of body) {
			if (ch === "{" || ch === "[" || ch === "(") depth++;
			else if (ch === "}" || ch === "]" || ch === ")") depth--;
			if (depth === 0 && ch === ",") {
				flush();
				segment = "";
			} else {
				segment += ch;
			}
		}
		const active = /^\s*([A-Za-z_$][\w$]*)\s*:([\s\S]*)$/.exec(segment);
		if (active) {
			activeKey = active[1];
			existingKeys.push(active[1]);
		}
	}

	const resolution = resolveCallHead(call.head, env, existingKeys);
	const info = resolution.info;
	const receiverBound = receiverIsBound(resolution);
	const bindable = info
		? info.args.filter((arg) => !(receiverBound && arg === info.receiver))
		: [];
	const base = {
		callName: call.head.display,
		info,
		candidates: resolution.candidates,
		receiverBound,
	};

	if (braceOpen === undefined) {
		const active = pieces.length - 1;
		return {
			...base,
			positionalCount: active,
			params: bindable.slice(active),
			existingKeys: [],
			mode: "positional",
		};
	}

	const positionalCount = pieces.length - 1;
	const params = bindable.slice(positionalCount);
	if (activeKey) {
		return {
			...base,
			positionalCount,
			params,
			existingKeys,
			mode: "value",
			activeArg: activeKey,
		};
	}
	return { ...base, positionalCount, params, existingKeys, mode: "key" };
}

function snippetPlaceholder(arg: FlowScriptArg, tabStop: number): string {
	if (arg.enumValues && arg.enumValues.length > 0) {
		return `"\${${tabStop}|${arg.enumValues.join(",")}|}"`;
	}
	return `\${${tabStop}:${arg.typeString}}`;
}

function argsSnippet(args: FlowScriptArg[]): string {
	if (args.length === 0) return "()";
	const params = args
		.map((arg, idx) => `${arg.name}: ${snippetPlaceholder(arg, idx + 1)}`)
		.join(", ");
	return `({ ${params} })`;
}

/** Static call snippet in the given spelling (qualified by default, bare when opened by `use`). */
export function buildCallSnippet(
	info: FlowScriptNodeInfo,
	name: string,
): string {
	return `${name}${argsSnippet(info.args)}`;
}

/** Method-form snippet: receiver already bound; a single remaining input is passed positionally. */
function buildMethodSnippet(info: FlowScriptNodeInfo): string {
	const rest = methodParams(info);
	const name = info.alias ?? info.identifier;
	if (rest.length === 1) return `${name}(${snippetPlaceholder(rest[0], 1)})`;
	return `${name}${argsSnippet(rest)}`;
}

const CACHE_DECORATOR_MARKDOWN = `\`\`\`flowscript
@cache
@cache({})
@cache({ namespace: "pricing", ttlSeconds: 0, scope: "user" })
\`\`\`

Caches a function's outputs by its layer and inputs. A cache hit replays the outputs and skips the entire function body, including side effects.

Bare \`@cache\` and \`@cache({})\` use the \`"global"\` namespace, a 300-second lifetime, and app scope. Set \`ttlSeconds: 0\` explicitly for entries that remain until invalidated. Use user scope for private or user-dependent results. The decorator only applies to \`function\` declarations.`;

const CACHE_FIELD_MARKDOWN: Record<string, string> = {
	namespace:
		'`namespace: string` — Groups cache entries for invalidation. Defaults to `"global"`.',
	ttlSeconds:
		"`ttlSeconds: int` — Non-negative cache lifetime in seconds. Defaults to `300`; use `0` for no expiry.",
	scope:
		'`scope: "app" | "user"` — `app` shares entries across the app; `user` isolates entries by triggering user.',
};

interface CacheDecoratorContext {
	existingKeys: string[];
	mode: "key" | "value";
	activeField?: string;
}

/** Detects the cursor inside the settings object of an unfinished `@cache({ ... })`. */
export function analyzeCacheDecoratorContext(
	maskedBefore: string,
): CacheDecoratorContext | null {
	const head = /@cache\s*\(\s*\{/g;
	let match: RegExpExecArray | null = null;
	for (
		let candidate = head.exec(maskedBefore);
		candidate;
		candidate = head.exec(maskedBefore)
	) {
		match = candidate;
	}
	if (!match) return null;

	const body = maskedBefore.slice(match.index + match[0].length);
	const existingKeys: string[] = [];
	let depth = 0;
	let segment = "";
	const flush = () => {
		const field = /^\s*([A-Za-z_$][\w$]*)\s*:/.exec(segment);
		if (field) existingKeys.push(field[1]);
	};

	for (const ch of body) {
		if (ch === "{" || ch === "[" || ch === "(") {
			depth++;
		} else if (ch === "}" || ch === "]" || ch === ")") {
			if (depth === 0) return null;
			depth--;
		}
		if (depth === 0 && ch === ",") {
			flush();
			segment = "";
		} else {
			segment += ch;
		}
	}

	const active = /^\s*([A-Za-z_$][\w$]*)\s*:([\s\S]*)$/.exec(segment);
	if (active) {
		existingKeys.push(active[1]);
		return {
			existingKeys,
			mode: "value",
			activeField: active[1],
		};
	}
	return { existingKeys, mode: "key" };
}

interface CompletionRange {
	startLineNumber: number;
	endLineNumber: number;
	startColumn: number;
	endColumn: number;
}

/** Completion items for the members and nested namespaces of one namespace (after `ns::`). */
function namespaceMemberItems(
	monaco: Monaco,
	ns: FlowScriptNamespace,
	range: CompletionRange,
): unknown[] {
	const items: unknown[] = [];
	for (const [alias, info] of ns.members) {
		items.push({
			label: { label: alias, description: info.friendlyName },
			kind: monaco.languages.CompletionItemKind.Function,
			detail: renderSignature(info),
			documentation: { value: nodeHoverMarkdown(info) },
			insertText: buildCallSnippet(info, alias),
			insertTextRules:
				monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
			filterText: `${alias} ${info.identifier} ${info.friendlyName}`,
			range,
			sortText: `0_${alias}`,
		});
	}
	for (const [segment, child] of ns.children) {
		items.push({
			label: segment,
			kind: monaco.languages.CompletionItemKind.Module,
			detail: `namespace ${child.key} (${child.members.size} members)`,
			insertText: `${segment}::`,
			command: { id: "editor.action.triggerSuggest", title: "Suggest" },
			range,
			sortText: `1_${segment}`,
		});
	}
	return items;
}

interface CachedCompletionEntry {
	/** Set when the item stands for a catalog node (skipped if already listed by `use`). */
	info?: FlowScriptNodeInfo;
	/** Alias exclusion key for method items. */
	alias?: string;
	item: Record<string, unknown>;
}

interface CatalogCompletionCache {
	monaco: Monaco;
	statics: CachedCompletionEntry[];
	methodsByClass: Map<string, CachedCompletionEntry[]>;
}

/**
 * Doc-independent completion items (every catalog node, top-level namespaces, keywords,
 * types, constants, and the per-class method lists) are built once per catalog index —
 * completion fires on nearly every keystroke, and rebuilding ~1,700 documented items each
 * time dominated its cost. Items are cached without a `range`; callers spread one in.
 */
const catalogCompletionCaches = new WeakMap<
	FlowScriptIndex,
	CatalogCompletionCache
>();

function catalogCompletionCache(
	monaco: Monaco,
	index: FlowScriptIndex,
): CatalogCompletionCache {
	const cached = catalogCompletionCaches.get(index);
	if (cached && cached.monaco === monaco) return cached;
	const statics: CachedCompletionEntry[] = [];
	for (const info of index.byName.values()) {
		const name = displayName(info);
		statics.push({
			info,
			item: {
				label: { label: name, description: info.friendlyName },
				kind: monaco.languages.CompletionItemKind.Function,
				detail: renderSignature(info),
				documentation: { value: nodeHoverMarkdown(info) },
				insertText: buildCallSnippet(info, name),
				insertTextRules:
					monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
				filterText: `${name} ${info.alias ?? ""} ${info.identifier} ${info.friendlyName}`,
				sortText: `2_${name}`,
			},
		});
	}
	for (const ns of index.namespaces.values()) {
		if (ns.path.length !== 1) continue;
		statics.push({
			item: {
				label: ns.key,
				kind: monaco.languages.CompletionItemKind.Module,
				detail: `namespace (${ns.members.size} members${
					ns.children.size > 0 ? `, ${ns.children.size} nested` : ""
				})`,
				insertText: `${ns.key}::`,
				command: { id: "editor.action.triggerSuggest", title: "Suggest" },
				sortText: `2_${ns.key}::`,
			},
		});
	}
	for (const keyword of [...STORAGE_KEYWORDS, ...CONTROL_KEYWORDS]) {
		statics.push({
			item: {
				label: keyword,
				kind: monaco.languages.CompletionItemKind.Keyword,
				insertText: keyword,
				sortText: `3_${keyword}`,
			},
		});
	}
	for (const type of TYPE_KEYWORDS) {
		statics.push({
			item: {
				label: type,
				kind: monaco.languages.CompletionItemKind.TypeParameter,
				insertText: type,
				sortText: `4_${type}`,
			},
		});
	}
	for (const constant of CONSTANTS) {
		statics.push({
			item: {
				label: constant,
				kind: monaco.languages.CompletionItemKind.Constant,
				insertText: constant,
				sortText: `4_${constant}`,
			},
		});
	}
	const built: CatalogCompletionCache = {
		monaco,
		statics,
		methodsByClass: new Map(),
	};
	catalogCompletionCaches.set(index, built);
	return built;
}

const METHOD_ITEMS_ANY_CLASS = " *";

function methodItemsForClass(
	monaco: Monaco,
	index: FlowScriptIndex,
	cls: string | undefined,
): CachedCompletionEntry[] {
	const cache = catalogCompletionCache(monaco, index);
	const key = cls ?? METHOD_ITEMS_ANY_CLASS;
	const cached = cache.methodsByClass.get(key);
	if (cached) return cached;
	const items: CachedCompletionEntry[] = [];
	const seen = new Set<FlowScriptNodeInfo>();
	const push = (info: FlowScriptNodeInfo, priority: string, group?: string) => {
		if (seen.has(info) || !info.alias) return;
		seen.add(info);
		items.push({
			info,
			alias: info.alias,
			item: {
				label: {
					label: info.alias,
					description: group ? `${group} · ${info.qualified}` : info.qualified,
				},
				kind: monaco.languages.CompletionItemKind.Method,
				detail: renderSignature(info, true),
				documentation: { value: nodeHoverMarkdown(info) },
				insertText: buildMethodSnippet(info),
				insertTextRules:
					monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
				filterText: `${info.alias} ${info.identifier} ${info.friendlyName}`,
				sortText: `${priority}_${info.alias}`,
			},
		});
	};
	const { methods } = index;
	if (cls) {
		for (const bucket of methods.get(cls)?.values() ?? [])
			for (const info of bucket) push(info, "1");
		if (isTitledStruct(cls))
			for (const bucket of methods.get("struct")?.values() ?? [])
				for (const info of bucket) push(info, "2");
		for (const bucket of methods.get(UNIVERSAL_CLASS)?.values() ?? [])
			for (const info of bucket) push(info, "3");
	} else {
		for (const [group, table] of methods)
			for (const bucket of table.values())
				for (const info of bucket) push(info, "5", group);
	}
	cache.methodsByClass.set(key, items);
	return items;
}

/** Method completions for a receiver class (all classes, lower priority, when unknown). */
function methodItems(
	monaco: Monaco,
	cls: string | undefined,
	env: TypeEnv,
	range: CompletionRange,
	exclude: Set<string>,
): unknown[] {
	const items: unknown[] = [];
	for (const entry of methodItemsForClass(monaco, env.index, cls)) {
		if (entry.alias && exclude.has(entry.alias)) continue;
		items.push({ ...entry.item, range });
	}
	for (const [name, receiverClass] of env.symbols.functionReceivers) {
		if (exclude.has(name)) continue;
		if (cls && receiverClass && receiverClass !== cls) continue;
		items.push({
			label: { label: name, description: "function (this board)" },
			kind: monaco.languages.CompletionItemKind.Method,
			detail: receiverClass ? `${receiverClass}.${name}(…)` : `${name}(…)`,
			insertText: `${name}($1)`,
			insertTextRules:
				monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
			range,
			sortText: `1_${name}`,
		});
	}
	return items;
}

/**
 * Runs `compute` against the document's environment: hydrated from the language worker
 * for large documents (analysis leaves the UI thread), synchronously in-thread otherwise
 * (SSR, tests, small documents). A cancelled worker request resolves to `null`.
 */
function withEnvDoc<T>(
	model: { uri: unknown; getValue: () => string; getVersionId?: () => number },
	getCatalogNodes: () => INode[] | undefined,
	token: CancellationTokenLike | undefined,
	compute: (envDoc: FlowScriptEnvDoc, index: FlowScriptIndex) => T,
): T | Promise<T | null> {
	const nodes = getCatalogNodes();
	const index = getFlowScriptIndex(nodes);
	const viaWorker = requestFlowScriptWorkerEnvDoc(model, nodes, token);
	if (!viaWorker)
		return compute(getFlowScriptEnvDoc(model.getValue(), index), index);
	return viaWorker.then((outcome) => {
		if (outcome.status === "cancelled") return null;
		const envDoc =
			outcome.status === "ok"
				? outcome.value
				: getFlowScriptEnvDoc(model.getValue(), index);
		return compute(envDoc, index);
	});
}

/**
 * Registers every FlowScript language provider: completion, hover and signature help here,
 * plus the analysis-backed features (code actions, auto-import, outline, folding, snippets,
 * inlay hints, definition/references, semantic tokens, rename) from the sibling module —
 * all backed by the live catalog. Returns a single disposable that tears every provider down.
 */
export function registerFlowScriptProviders(
	monaco: Monaco,
	getCatalogNodes: () => INode[] | undefined,
): { dispose: () => void } {
	const completion = monaco.languages.registerCompletionItemProvider(
		FLOWSCRIPT_LANGUAGE_ID,
		{
			triggerCharacters: [".", ":", "{", ",", " ", "@"],
			provideCompletionItems: (model, position, _context, token) =>
				withEnvDoc(model, getCatalogNodes, token, (envDoc, index) => {
					const word = model.getWordUntilPosition(position);
					const range = {
						startLineNumber: position.lineNumber,
						endLineNumber: position.lineNumber,
						startColumn: word.startColumn,
						endColumn: word.endColumn,
					};

					const maskedFull = envDoc.masked;
					const offset = model.getOffsetAt(position);
					const maskedBefore = maskedFull.slice(0, offset);
					const decoratorToken = /@[A-Za-z_]*$/.exec(maskedBefore);
					if (decoratorToken) {
						const decoratorRange = {
							...range,
							startColumn: position.column - decoratorToken[0].length,
						};
						return {
							suggestions: [
								{
									label: "@cache",
									kind: monaco.languages.CompletionItemKind.Keyword,
									detail: "Enable function result caching with defaults",
									documentation: { value: CACHE_DECORATOR_MARKDOWN },
									insertText: "@cache",
									range: decoratorRange,
									sortText: "0_cache_bare",
								},
								{
									label: "@cache({ … })",
									kind: monaco.languages.CompletionItemKind.Keyword,
									detail: "Configure function result caching",
									documentation: { value: CACHE_DECORATOR_MARKDOWN },
									insertText:
										'@cache({ namespace: "${1:global}", ttlSeconds: ${2:300}, scope: "${3|app,user|}" })',
									insertTextRules:
										monaco.languages.CompletionItemInsertTextRule
											.InsertAsSnippet,
									range: decoratorRange,
									sortText: "0_cache_configured",
								},
								{
									label: "@parallel",
									kind: monaco.languages.CompletionItemKind.Keyword,
									detail: "Run the following for…of loop body in parallel",
									insertText: "@parallel",
									range: decoratorRange,
									sortText: "1_parallel",
								},
							],
						};
					}

					const cacheContext = analyzeCacheDecoratorContext(maskedBefore);
					if (cacheContext?.mode === "value") {
						if (cacheContext.activeField !== "scope") {
							return { suggestions: [] };
						}
						const quotedValue = /"[^"\n]*$/.exec(envDoc.text.slice(0, offset));
						const scopeRange = quotedValue
							? {
									...range,
									startColumn: position.column - quotedValue[0].length,
								}
							: range;
						return {
							suggestions: ["app", "user"].map((scope) => ({
								label: `"${scope}"`,
								kind: monaco.languages.CompletionItemKind.EnumMember,
								detail:
									scope === "app"
										? "Shared across the app"
										: "Isolated by triggering user",
								insertText: `"${scope}"`,
								range: scopeRange,
								sortText: `0_${scope}`,
							})),
						};
					}
					if (cacheContext?.mode === "key") {
						const present = new Set(cacheContext.existingKeys);
						const fields = [
							{
								name: "namespace",
								detail: "string (optional)",
								insertText: 'namespace: "${1:global}"',
							},
							{
								name: "ttlSeconds",
								detail: "non-negative integer (optional)",
								insertText: "ttlSeconds: ${1:300}",
							},
							{
								name: "scope",
								detail: '"app" | "user" (optional)',
								insertText: 'scope: "${1|app,user|}"',
							},
						];
						return {
							suggestions: fields
								.filter((field) => !present.has(field.name))
								.map((field) => ({
									label: field.name,
									kind: monaco.languages.CompletionItemKind.Field,
									detail: field.detail,
									documentation: {
										value: CACHE_FIELD_MARKDOWN[field.name],
									},
									insertText: field.insertText,
									insertTextRules:
										monaco.languages.CompletionItemInsertTextRule
											.InsertAsSnippet,
									range,
									sortText: `0_${field.name}`,
								})),
						};
					}

					const env = envDoc.env;
					const beforeWord = stripTrailingWord(maskedBefore).trimEnd();

					// Path position (`string::`, `ai::ml::`) → members and nested namespaces.
					if (beforeWord.endsWith("::")) {
						const pathMatch = new RegExp(`((?:${IDENT_SRC}\\s*::\\s*)+)$`).exec(
							beforeWord,
						);
						const segments = pathMatch
							? pathMatch[1].split(PATH_SPLIT_RE).filter(Boolean)
							: [];
						const ns = env.index.namespaces.get(
							namespaceKey(expandPath(segments, env.scope)),
						);
						return {
							suggestions: (ns
								? namespaceMemberItems(monaco, ns, range)
								: []) as never[],
						};
					}

					// Dot notation → members (output pins / struct fields, following bindings and
					// nested $refs) plus the methods callable on the receiver's class.
					if (beforeWord.endsWith(".")) {
						const receiverExpr = extractTrailingExpr(beforeWord.slice(0, -1));
						const receiver = evaluateExpr(receiverExpr, env);
						if (receiver.namespace) {
							const ns = env.index.namespaces.get(
								namespaceKey(receiver.namespace),
							);
							return {
								suggestions: (ns
									? namespaceMemberItems(monaco, ns, range)
									: []) as never[],
							};
						}
						const members: {
							name: string;
							typeString: string;
							description: string;
						}[] = [];
						if (receiver.node && receiver.node.outputs.length > 1)
							members.push(
								...sourceMembers({ kind: "node", info: receiver.node }),
							);
						if (receiver.source?.kind === "struct")
							members.push(...sourceMembers(receiver.source));
						const memberNames = new Set(members.map((member) => member.name));
						const suggestions: unknown[] = members.map((member) => ({
							label: member.name,
							kind: monaco.languages.CompletionItemKind.Property,
							detail: member.typeString,
							documentation: member.description
								? { value: member.description }
								: undefined,
							insertText: member.name,
							range,
							sortText: `0_${member.name}`,
						}));
						if (receiver.value?.isArray && !memberNames.has("length")) {
							memberNames.add("length");
							suggestions.push({
								label: "length",
								kind: monaco.languages.CompletionItemKind.Property,
								detail: "int",
								insertText: "length",
								range,
								sortText: "0_length",
							});
						}
						suggestions.push(
							...methodItems(
								monaco,
								classOfValue(receiver.value),
								env,
								range,
								memberNames,
							),
						);
						return { suggestions: suggestions as never[] };
					}

					const context = analyzeContext(maskedBefore, env);

					// Enum argument value → offer the allowed literals only.
					if (context?.info) {
						const activeArg =
							context.mode === "value" && context.activeArg
								? context.info.args.find(
										(candidate) => candidate.name === context.activeArg,
									)
								: context.mode === "positional"
									? context.params[0]
									: undefined;
						if (activeArg?.enumValues && activeArg.enumValues.length > 0) {
							return {
								suggestions: activeArg.enumValues.map((value) => ({
									label: `"${value}"`,
									kind: monaco.languages.CompletionItemKind.EnumMember,
									insertText: `"${value}"`,
									range,
									sortText: `0_${value}`,
								})),
							};
						}
						// Non-enum value → fall through to variables / functions / constants.
					}

					// Key position inside a known call → offer the remaining argument names.
					if (context?.mode === "key" && context.info) {
						const keyInfo = context.info;
						const present = new Set(context.existingKeys);
						return {
							suggestions: context.params
								.filter((arg) => !present.has(arg.name))
								.map((arg) => ({
									label: { label: arg.name, description: arg.friendlyName },
									kind: monaco.languages.CompletionItemKind.Field,
									detail: `${arg.typeString}${arg.optional ? " (optional)" : ""}`,
									documentation: { value: argHoverMarkdown(keyInfo, arg) },
									insertText: `${arg.name}: `,
									filterText: `${arg.name} ${arg.friendlyName}`,
									range,
									sortText: `${arg.optional ? "1" : "0"}_${arg.name}`,
								})),
						};
					}

					// Default: document symbols, node calls, namespaces, keywords, types and constants.
					const suggestions: unknown[] = [];
					const symbols = env.symbols;
					for (const [name, type] of symbols.variables) {
						if (index.byName.has(name)) continue;
						suggestions.push({
							label: name,
							kind: monaco.languages.CompletionItemKind.Variable,
							detail: type ? `variable: ${type}` : "variable",
							insertText: name,
							range,
							sortText: `1_${name}`,
						});
					}
					for (const name of symbols.functions) {
						if (index.byName.has(name)) continue;
						suggestions.push({
							label: name,
							kind: monaco.languages.CompletionItemKind.Function,
							detail: "function (this board)",
							insertText: name,
							range,
							sortText: `1_${name}`,
						});
					}
					for (const name of symbols.interfaces) {
						if (index.byName.has(name)) continue;
						suggestions.push({
							label: name,
							kind: monaco.languages.CompletionItemKind.Interface,
							detail: "interface",
							insertText: name,
							range,
							sortText: `1_${name}`,
						});
					}
					const listed = new Set<FlowScriptNodeInfo>();
					for (const bucket of env.scope.openMembers.values()) {
						for (const info of bucket) {
							if (listed.has(info) || !info.alias) continue;
							listed.add(info);
							suggestions.push({
								label: { label: info.alias, description: info.qualified },
								kind: monaco.languages.CompletionItemKind.Function,
								detail: renderSignature(info),
								documentation: { value: nodeHoverMarkdown(info) },
								insertText: buildCallSnippet(info, info.alias),
								insertTextRules:
									monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
								filterText: `${info.alias} ${info.identifier} ${info.friendlyName}`,
								range,
								sortText: `1_${info.alias}`,
							});
						}
					}
					for (const entry of catalogCompletionCache(monaco, index).statics) {
						if (entry.info && listed.has(entry.info)) continue;
						suggestions.push({ ...entry.item, range });
					}
					return { suggestions: suggestions as never[] };
				}),
		},
	);

	const hover = monaco.languages.registerHoverProvider(FLOWSCRIPT_LANGUAGE_ID, {
		provideHover: (model, position, token) => {
			const word = model.getWordAtPosition(position);
			if (!word) return null;
			return withEnvDoc(model, getCatalogNodes, token, (envDoc) => {
				const range = {
					startLineNumber: position.lineNumber,
					endLineNumber: position.lineNumber,
					startColumn: word.startColumn,
					endColumn: word.endColumn,
				};
				const value = envDoc.text;
				const wordStartOffset = model.getOffsetAt({
					lineNumber: position.lineNumber,
					column: word.startColumn,
				});
				if (
					word.word === "cache" &&
					value.slice(0, wordStartOffset).endsWith("@")
				) {
					return { range, contents: [{ value: CACHE_DECORATOR_MARKDOWN }] };
				}
				if (
					CACHE_FIELD_MARKDOWN[word.word] &&
					analyzeCacheDecoratorContext(
						maskLiterals(value.slice(0, wordStartOffset)),
					)
				) {
					return {
						range,
						contents: [{ value: CACHE_FIELD_MARKDOWN[word.word] }],
					};
				}

				const maskedFull = envDoc.masked;
				const env = envDoc.env;
				const lineBefore = model.getValueInRange({
					startLineNumber: position.lineNumber,
					startColumn: 1,
					endLineNumber: position.lineNumber,
					endColumn: word.startColumn,
				});
				const lineAfter = model.getValueInRange({
					startLineNumber: position.lineNumber,
					startColumn: word.endColumn,
					endLineNumber: position.lineNumber,
					endColumn: Number.MAX_SAFE_INTEGER,
				});
				const maskedLine = maskLiterals(lineBefore).trimEnd();
				const followedByPathSep = /^\s*::/.test(lineAfter);
				const followedByCall = /^\s*\(/.test(lineAfter);

				// Qualified spelling: `hash::md5`, or a namespace segment of it.
				if (maskedLine.endsWith("::") || followedByPathSep) {
					const pathMatch = new RegExp(`((?:${IDENT_SRC}\\s*::\\s*)*)$`).exec(
						maskedLine,
					);
					const prefix = pathMatch
						? pathMatch[1].split(PATH_SPLIT_RE).filter(Boolean)
						: [];
					if (followedByPathSep) {
						const ns = env.index.namespaces.get(
							namespaceKey(expandPath([...prefix, word.word], env.scope)),
						);
						if (ns)
							return {
								range,
								contents: [{ value: namespaceHoverMarkdown(ns) }],
							};
					} else {
						const info = resolvePathCall(prefix, word.word, env).info;
						if (info)
							return { range, contents: [{ value: nodeHoverMarkdown(info) }] };
					}
				}

				// Method call: `s.trim(` → the node dispatched on the receiver's class.
				if (maskedLine.endsWith(".") && followedByCall) {
					const receiver = evaluateExpr(
						extractTrailingExpr(maskedLine.slice(0, -1)),
						env,
					);
					const resolution = receiver.namespace
						? resolvePathCall(receiver.namespace, word.word, env)
						: resolveMethod(receiver, word.word, env);
					if (resolution.info)
						return {
							range,
							contents: [{ value: nodeHoverMarkdown(resolution.info) }],
						};
				}

				if (!maskedLine.endsWith(".")) {
					const bare = resolveBareCall(word.word, env);
					if (bare.info) {
						return {
							range,
							contents: [{ value: nodeHoverMarkdown(bare.info) }],
						};
					}
					const ns = env.index.namespaces.get(
						namespaceKey(expandPath([word.word], env.scope)),
					);
					if (ns && !isBoundName(word.word, env))
						return { range, contents: [{ value: namespaceHoverMarkdown(ns) }] };
				}

				// Member access on a variable (`result.value`) → describe the source node's output pin.
				if (maskedLine.endsWith(".")) {
					const receiver = evaluateExpr(
						extractTrailingExpr(maskedLine.slice(0, -1)),
						env,
					);
					const source: MemberSource | null =
						receiver.node && receiver.node.outputs.length > 1
							? { kind: "node", info: receiver.node }
							: receiver.source;
					const member = source
						? sourceMembers(source).find((m) => m.name === word.word)
						: undefined;
					if (source && member) {
						const owner =
							source.kind === "node"
								? `\`${displayName(source.info)}\``
								: source.title
									? `\`${source.title}\``
									: "struct";
						const kind = source.kind === "node" ? "output" : "field";
						const lines = [
							`\`${member.name}: ${member.typeString}\` — ${kind} of ${owner}`,
						];
						if (member.description) lines.push(member.description);
						return { range, contents: [{ value: lines.join("\n\n") }] };
					}
				}

				const maskedBefore = maskedFull.slice(0, model.getOffsetAt(position));
				const context = analyzeContext(maskedBefore, env);
				if (context?.info) {
					const arg = context.info.args.find(
						(candidate) => candidate.name === word.word,
					);
					if (arg) {
						return {
							range,
							contents: [{ value: argHoverMarkdown(context.info, arg) }],
						};
					}
				}
				return null;
			});
		},
	});

	const signature = monaco.languages.registerSignatureHelpProvider(
		FLOWSCRIPT_LANGUAGE_ID,
		{
			signatureHelpTriggerCharacters: ["(", "{", ",", ":"],
			signatureHelpRetriggerCharacters: [",", ":"],
			provideSignatureHelp: (model, position, token) =>
				withEnvDoc(model, getCatalogNodes, token, (envDoc) => {
					const context = analyzeContext(
						envDoc.masked.slice(0, model.getOffsetAt(position)),
						envDoc.env,
					);
					if (!context?.info) return null;
					const info = context.info;
					const params = context.receiverBound ? methodParams(info) : info.args;
					if (params.length === 0) return null;

					let activeParameter = 0;
					if (context.mode === "positional") {
						activeParameter = Math.min(
							context.positionalCount,
							params.length - 1,
						);
					} else if (context.mode === "value" && context.activeArg) {
						const idx = params.findIndex(
							(arg) => arg.name === context.activeArg,
						);
						if (idx >= 0) activeParameter = idx;
					} else {
						const present = new Set(context.existingKeys);
						const next = params.findIndex(
							(arg, idx) =>
								idx >= context.positionalCount && !present.has(arg.name),
						);
						activeParameter = next >= 0 ? next : params.length - 1;
					}

					return {
						value: {
							signatures: [
								{
									label: renderSignature(info, context.receiverBound),
									documentation: { value: info.description },
									parameters: params.map((arg) => ({
										label: argSignature(arg),
										documentation: { value: argHoverMarkdown(info, arg) },
									})),
								},
							],
							activeSignature: 0,
							activeParameter,
						},
						dispose: () => {},
					};
				}),
		},
	);

	const features = registerFlowScriptFeatureProviders(monaco, getCatalogNodes);

	return {
		dispose: () => {
			completion.dispose();
			hover.dispose();
			signature.dispose();
			features.dispose();
		},
	};
}

interface RawMarker {
	message: string;
	start: number;
	end: number;
	severity: "error" | "warning";
}

/** Collects declared function/event names so calls to user code are not flagged as unknown. */
function collectDeclaredNames(text: string): Set<string> {
	const names = new Set<string>();
	const declared = /\b(?:function|event)\s+([A-Za-z_$][\w$]*)/g;
	for (let m = declared.exec(text); m; m = declared.exec(text)) names.add(m[1]);
	for (const name of collectEventHeaderNames(text)) names.add(name);
	return names;
}

export interface ArgLiteral {
	name: string;
	start: number;
	value: string;
	valueStart: number;
}

export interface CallArgs {
	positional: { value: string; start: number }[];
	/** The trailing `{ key: value, … }` object, when present. */
	named: ArgLiteral[] | null;
}

/** Parses a `{ key: value, … }` object starting at `braceIndex` into top-level key/value pairs. */
function readNamedArgs(masked: string, braceIndex: number): ArgLiteral[] {
	let i = braceIndex + 1;
	const args: ArgLiteral[] = [];
	while (i < masked.length) {
		while (i < masked.length && (/\s/.test(masked[i]) || masked[i] === ","))
			i++;
		if (i >= masked.length || masked[i] === "}") break;
		if (!IDENT_CHAR.test(masked[i])) {
			i++;
			continue;
		}
		const keyStart = i;
		while (i < masked.length && IDENT_CHAR.test(masked[i])) i++;
		const name = masked.slice(keyStart, i);
		while (i < masked.length && /\s/.test(masked[i])) i++;
		if (masked[i] !== ":") continue;
		i++;
		while (i < masked.length && /\s/.test(masked[i])) i++;
		const valueStart = i;
		let vdepth = 0;
		while (i < masked.length) {
			const c = masked[i];
			if (c === "{" || c === "[" || c === "(") vdepth++;
			else if (c === "}" || c === "]" || c === ")") {
				if (vdepth === 0) break;
				vdepth--;
			} else if (c === "," && vdepth === 0) break;
			i++;
		}
		args.push({
			name,
			start: keyStart,
			value: masked.slice(valueStart, i).trim(),
			valueStart,
		});
	}
	return args;
}

/**
 * Parses a call's argument list: positional values, then the named-args object when a `{ … }`
 * sits in the last slot (a `{ … }` anywhere earlier is a positional struct literal).
 */
export function readCallArgs(
	masked: string,
	parenIndex: number,
): CallArgs | null {
	const close = matchBracket(masked, parenIndex);
	if (close < 0) return null;
	const pieces = splitTopLevel(
		masked.slice(parenIndex + 1, close),
		parenIndex + 1,
	).filter((piece, idx, all) => piece.text.trim() || idx < all.length - 1);
	const positional: CallArgs["positional"] = [];
	let named: ArgLiteral[] | null = null;
	pieces.forEach((piece, idx) => {
		const text = piece.text.trim();
		if (!text) return;
		const start =
			piece.start + (piece.text.length - piece.text.trimStart().length);
		if (idx === pieces.length - 1 && text.startsWith("{")) {
			named = readNamedArgs(masked, start);
		} else {
			positional.push({ value: text, start });
		}
	});
	return { positional, named };
}

/** Builds a diagnostic message, upgrading to a helpful hint when a multi-output bundle is misused. */
function describeMismatch(
	subject: string,
	actual: ValueType,
	reason: string,
): string {
	if (actual.multiOutput) {
		const { node, outputs } = actual.multiOutput;
		const hint = outputs[0] ? ` — access one, e.g. \`.${outputs[0]}\`` : "";
		return `${subject}: '${node}' returns multiple values (${outputs.join(", ")})${hint}`;
	}
	return `${subject}: ${reason}`;
}

function typeCompatibility(
	expected: ValueType,
	actual: ValueType,
): string | null {
	if (expected.group === "any") return null;
	// A bare multi-output call result is a bundle, never a usable value on its own.
	if (actual.multiOutput) return "returns multiple values";
	if (actual.group === "any" || actual.group === "null") return null;
	if (expected.isArray !== actual.isArray) {
		return expected.isArray
			? "expected an array"
			: "expected a single value, got an array";
	}
	if (expected.group !== actual.group) {
		// Date/Path/bytes are commonly authored as string literals.
		if (
			(expected.group === "date" ||
				expected.group === "path" ||
				expected.group === "bytes") &&
			actual.group === "string"
		)
			return null;
		return `expected ${expected.group}, got ${actual.group}`;
	}
	if (
		expected.group === "struct" &&
		expected.schemaTitle &&
		actual.schemaTitle &&
		expected.schemaTitle !== actual.schemaTitle
	) {
		return `expected schema '${expected.schemaTitle}', got '${actual.schemaTitle}'`;
	}
	return null;
}

/** Lowercased alias/flat name → hint spellings, built once per catalog index. */
const spellingHintTables = new WeakMap<
	FlowScriptIndex,
	Map<string, string[]>
>();

function spellingHintTable(index: FlowScriptIndex): Map<string, string[]> {
	let table = spellingHintTables.get(index);
	if (table) return table;
	table = new Map();
	const push = (key: string, hint: string) => {
		const bucket = table?.get(key);
		if (!bucket) table?.set(key, [hint]);
		else if (!bucket.includes(hint)) bucket.push(hint);
	};
	for (const info of index.byQualified.values()) {
		if (info.alias && info.qualified)
			push(info.alias.toLowerCase(), `${info.qualified}(…)`);
	}
	for (const [flat, info] of index.byName) {
		push(flat.toLowerCase(), `${displayName(info)}(…)`);
	}
	spellingHintTables.set(index, table);
	return table;
}

/** "Did you mean …" for an unknown call, from nodes with the same alias or flat name. */
function spellingHint(member: string, env: TypeEnv): string {
	const hints = spellingHintTable(env.index).get(member.toLowerCase());
	if (!hints || hints.length === 0) return "";
	return ` Did you mean ${hints
		.slice(0, 3)
		.map((hint) => `\`${hint}\``)
		.join(" or ")}?`;
}

/** A positioned diagnostic with a plain severity, independent of the Monaco enums. */
export interface FlowScriptRawDiagnostic {
	message: string;
	severity: "error" | "warning";
	startLineNumber: number;
	startColumn: number;
	endLineNumber: number;
	endColumn: number;
}

/**
 * Conservative client-side structural linter: unknown function calls (flat, qualified or method
 * spelling), unknown/duplicate argument keys, positional overflow, unknown `use` namespaces, and
 * best-effort type/schema mismatches (literal, variable or output value vs the pin's expected type;
 * struct schema titles only when both sides declare one). It only reports when both sides are
 * confidently known, skipping anything it cannot model to avoid false positives on valid syntax;
 * the authoritative parser runs server-side in the studio.
 *
 * A `board` scope keeps a modular board's cross-file calls quiet: paths rooted in one of the
 * board's modules are not catalog namespaces, and functions declared in another file of the same
 * board are not undeclared. Without it (single-file board, or no board context) nothing changes.
 *
 * Pure (no Monaco): also runs inside the FlowScript web worker.
 */
export function computeFlowScriptRawDiagnostics(
	text: string,
	index: FlowScriptIndex,
	board?: FlowScriptBoardScope,
): FlowScriptRawDiagnostic[] {
	if (index.names.length === 0) return [];

	const { masked, templateExprs, env } = getFlowScriptEnvDoc(text, index);
	const declared = collectDeclaredNames(text);
	const moduleRoots = boardModuleRoots(board);
	const boardFunctions = boardFunctionNames(board);
	const raw: RawMarker[] = [];
	const skipSpans: Span[] = [...templateExprs];

	// `use` lines: namespaces must exist (only checked once the catalog carries namespaces).
	for (const use of env.symbols.uses) {
		skipSpans.push({ start: use.start, end: use.end });
		if (use.kind === "invalid") {
			raw.push({
				message: use.error,
				start: use.start,
				end: use.end,
				severity: "error",
			});
			continue;
		}
		if (index.namespaces.size === 0) continue;
		if (moduleRoots.has(use.path[0])) continue;
		const path = expandPath(use.path, env.scope);
		const ns = index.namespaces.get(namespaceKey(path));
		if (!ns) {
			raw.push({
				message: `Unknown namespace '${namespaceKey(use.path)}'.`,
				start: use.start,
				end: use.end,
				severity: "error",
			});
			continue;
		}
		if (use.kind === "members") {
			for (const member of use.members) {
				if (!ns.members.has(member))
					raw.push({
						message: `'${member}' is not a member of namespace '${ns.key}'.`,
						start: use.start,
						end: use.end,
						severity: "warning",
					});
			}
		}
	}

	const callRe = new RegExp(`(${PATH_SRC})\\s*\\(`, "g");
	for (let match = callRe.exec(masked); match; match = callRe.exec(masked)) {
		const nameStart = match.index;
		if (inSpan(nameStart, skipSpans)) continue;
		const parenIndex = nameStart + match[0].length - 1;
		const head = callHeadBefore(masked, parenIndex);
		if (!head) continue;
		const headEnd = nameStart + match[1].length;
		const range = { start: nameStart, end: headEnd };

		const args = readCallArgs(masked, parenIndex);
		const resolution = resolveCallHead(
			head,
			env,
			args?.named?.map((arg) => arg.name) ?? [],
		);
		if (resolution.userFunction) continue;
		const info = resolution.info;
		if (!info) {
			if (head.kind === "bare" && declared.has(head.member)) continue;
			// Cross-file calls on a modular board: `checkout::payments::helper()` is a module path,
			// not a catalog namespace, and `helper()` may be declared in another file of this board.
			if (head.kind === "path" && moduleRoots.has(head.path[0])) continue;
			if (head.kind === "bare" && boardFunctions.has(head.member)) continue;
			let message: string;
			if (head.kind === "path") {
				message = resolution.namespaceKnown
					? `Unknown function '${head.display}'. '${head.member}' is not a member of namespace '${namespaceKey(resolution.path ?? head.path)}'.${spellingHint(head.member, env)}`
					: `Unknown function '${head.display}'. Namespace '${namespaceKey(head.path)}' is not in the catalog.${spellingHint(head.member, env)}`;
			} else if (head.kind === "method") {
				if (
					!resolution.receiverClass &&
					index.methods.size === 0 &&
					index.byName.size > 0
				) {
					continue; // No method tables yet (names snapshot still loading).
				}
				message = resolution.receiverClass
					? `Unknown method '${head.member}' on ${resolution.receiverClass}.${spellingHint(head.member, env)}`
					: `Unknown method '${head.member}'. No catalog node is callable as .${head.member}().${spellingHint(head.member, env)}`;
			} else {
				message = `Unknown function '${head.member}'. It is not a catalog node or a declared function.${spellingHint(head.member, env)}`;
			}
			raw.push({ message, ...range, severity: "warning" });
			continue;
		}
		if (resolution.candidates.length > 1) {
			if (head.kind === "bare") {
				raw.push({
					message: `'${head.member}' is ambiguous: ${resolution.candidates
						.map((c) => `\`${displayName(c)}\``)
						.join(", ")}. Write the qualified name.`,
					...range,
					severity: "warning",
				});
			}
			continue; // Method dispatch on an unknown receiver type: cannot validate arguments.
		}

		if (!args) continue;
		const receiverBound = receiverIsBound(resolution);
		const bindable = info.args.filter(
			(arg) => !(receiverBound && arg === info.receiver),
		);
		const label = receiverBound ? `.${head.member}()` : head.display;

		if (args.positional.length > bindable.length) {
			const overflow = args.positional[bindable.length];
			raw.push({
				message: `Too many positional arguments for '${label}': it takes at most ${bindable.length}.`,
				start: overflow.start,
				end: overflow.start + Math.max(overflow.value.length, 1),
				severity: "warning",
			});
		}
		const boundPositionally = new Set<string>();
		args.positional.forEach((positional, idx) => {
			const pin = bindable[idx];
			if (!pin) return;
			boundPositionally.add(pin.name);
			const actual = evaluateExpr(positional.value, env).value;
			if (!actual) return;
			const reason = typeCompatibility(pinValueType(pin), actual);
			if (reason) {
				raw.push({
					message: describeMismatch(
						`Type mismatch for '${pin.name}' of '${label}'`,
						actual,
						reason,
					),
					start: positional.start,
					end: positional.start + Math.max(positional.value.length, 1),
					severity: "warning",
				});
			}
		});

		if (!args.named) continue;
		const argsByName = new Map(info.args.map((arg) => [arg.name, arg]));
		const seen = new Set<string>();
		for (const arg of args.named) {
			const pin = argsByName.get(arg.name);
			if (!pin) {
				raw.push({
					message: `Unknown argument '${arg.name}' for '${label}'.`,
					start: arg.start,
					end: arg.start + arg.name.length,
					severity: "warning",
				});
				continue;
			}
			if (receiverBound && pin === info.receiver) {
				raw.push({
					message: `Argument '${arg.name}' is already bound by the receiver of '${label}'.`,
					start: arg.start,
					end: arg.start + arg.name.length,
					severity: "warning",
				});
				continue;
			}
			if (boundPositionally.has(arg.name)) {
				raw.push({
					message: `Argument '${arg.name}' is already bound positionally in '${label}'.`,
					start: arg.start,
					end: arg.start + arg.name.length,
					severity: "warning",
				});
				continue;
			}
			if (seen.has(arg.name)) {
				raw.push({
					message: `Duplicate argument '${arg.name}' for '${label}'.`,
					start: arg.start,
					end: arg.start + arg.name.length,
					severity: "warning",
				});
			}
			seen.add(arg.name);

			const actual = evaluateExpr(arg.value, env).value;
			if (actual) {
				const reason = typeCompatibility(pinValueType(pin), actual);
				if (reason) {
					raw.push({
						message: describeMismatch(
							`Type mismatch for '${arg.name}' of '${label}'`,
							actual,
							reason,
						),
						start: arg.valueStart,
						end: arg.valueStart + Math.max(arg.value.length, 1),
						severity: "warning",
					});
				}
			}
		}
	}

	// Assignment statements: reassigning a known-typed variable to an incompatible value —
	// including a bare multi-output call result (e.g. `x = node(...)` instead of `.output`).
	const assignRe = /(?:^|[\n;{}])[ \t]*([A-Za-z_$][\w$]*)[ \t]*=(?!=)/g;
	for (let m = assignRe.exec(masked); m; m = assignRe.exec(masked)) {
		const lhsType = env.docVars.get(m[1]);
		if (!lhsType) continue;
		let r = m.index + m[0].length;
		while (r < masked.length && /[ \t]/.test(masked[r])) r++;
		const rhsStart = r;
		let depth = 0;
		while (r < masked.length) {
			const c = masked[r];
			if (c === "{" || c === "[" || c === "(") depth++;
			else if (c === "}" || c === "]" || c === ")") {
				if (depth === 0) break;
				depth--;
			} else if (depth === 0 && (c === "\n" || c === ";")) break;
			r++;
		}
		const rhs = masked.slice(rhsStart, r).trim();
		const actual = evaluateExpr(rhs, env).value;
		if (!actual) continue;
		const reason = typeCompatibility(lhsType, actual);
		if (reason) {
			raw.push({
				message: describeMismatch(`Cannot assign to '${m[1]}'`, actual, reason),
				start: rhsStart,
				end: rhsStart + Math.max(rhs.length, 1),
				severity: "warning",
			});
		}
	}

	const lineStarts = computeLineStartOffsets(text);
	return raw.map((marker) => {
		const start = offsetToPosition(lineStarts, marker.start);
		const end = offsetToPosition(lineStarts, marker.end);
		return {
			message: marker.message,
			severity: marker.severity,
			startLineNumber: start.lineNumber,
			startColumn: start.column,
			endLineNumber: end.lineNumber,
			endColumn: end.column,
		};
	});
}

/** Maps raw diagnostics onto Monaco marker severities. */
export function flowScriptMarkersFromRaw(
	monaco: Monaco,
	raw: readonly FlowScriptRawDiagnostic[],
): unknown[] {
	return raw.map((marker) => ({
		...marker,
		severity:
			marker.severity === "error"
				? monaco.MarkerSeverity.Error
				: monaco.MarkerSeverity.Warning,
	}));
}

/** See {@link computeFlowScriptRawDiagnostics}; this wrapper applies the Monaco severities. */
export function computeFlowScriptDiagnostics(
	monaco: Monaco,
	text: string,
	nodes: INode[] | undefined,
	board?: FlowScriptBoardScope,
): { markers: unknown[] } {
	const raw = computeFlowScriptRawDiagnostics(
		text,
		getFlowScriptIndex(nodes),
		board,
	);
	return { markers: flowScriptMarkersFromRaw(monaco, raw) };
}

// The heavier language features (code actions, auto-import, outline, folding, snippets,
// inlay hints, definition/references, semantic tokens, rename) live in a sibling module
// built on the shared per-document analysis; re-exported here so consumers keep one entry.
export {
	analyzeFlowScriptDocument,
	type FlowScriptAnalysis,
	type FlowScriptBinding,
	type FlowScriptCallSite,
	type FlowScriptDeclaration,
	type FlowScriptOccurrence,
} from "./flowscript-language-features";
