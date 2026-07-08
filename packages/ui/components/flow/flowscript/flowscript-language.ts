import type { Monaco } from "@monaco-editor/react";
import type { INode, IPin } from "../../../lib/schema/flow/node";
import {
	IPinType,
	IValueType,
	IVariableType,
} from "../../../lib/schema/flow/pin";

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
];

const CONTROL_KEYWORDS = [
	"if",
	"else",
	"for",
	"of",
	"in",
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

const KEYWORD_SET = new Set([...STORAGE_KEYWORDS, ...CONTROL_KEYWORDS]);

/** Mirrors `to_camel_case` in packages/ast/src/text.rs so completions match rendered node names. */
export function toFlowScriptIdentifier(input: string): string {
	let out = "";
	let upcomingUpper = false;
	let first = true;
	for (const ch of input) {
		if (/[a-zA-Z0-9]/.test(ch)) {
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

export interface FlowScriptArg {
	name: string;
	friendlyName: string;
	description: string;
	typeString: string;
	dataType: IVariableType;
	container: IValueType;
	schemaTitle?: string;
	optional: boolean;
	enumValues?: string[];
	sensitive: boolean;
}

export interface FlowScriptOutput {
	name: string;
	typeString: string;
	description: string;
	dataType: IVariableType;
	container: IValueType;
	schemaTitle?: string;
}

export interface FlowScriptNodeInfo {
	identifier: string;
	friendlyName: string;
	description: string;
	docs?: string;
	category: string;
	impure: boolean;
	args: FlowScriptArg[];
	outputs: FlowScriptOutput[];
}

export interface FlowScriptIndex {
	byName: Map<string, FlowScriptNodeInfo>;
	names: string[];
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
		friendlyName: pin.friendly_name || pin.name,
		description: pin.description ?? "",
		typeString,
		dataType: pin.data_type,
		container: pin.value_type,
		schemaTitle: title,
		optional: pin.default_value != null,
		enumValues: pin.options?.valid_values ?? undefined,
		sensitive: pin.options?.sensitive === true,
	};
}

function buildNodeInfo(node: INode): FlowScriptNodeInfo {
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
			typeString: pinTypeString(pin),
			description: pin.description ?? "",
			dataType: pin.data_type,
			container: pin.value_type,
			schemaTitle: schemaTitle(pin.schema),
		}));
	const impure = pins.some((pin) => pin.data_type === IVariableType.Execution);
	return {
		identifier: toFlowScriptIdentifier(node.name),
		friendlyName: node.friendly_name || node.name,
		description: node.description ?? "",
		docs: node.docs ?? undefined,
		category: node.category ?? "",
		impure,
		args,
		outputs,
	};
}

function buildFlowScriptIndex(nodes: INode[]): FlowScriptIndex {
	const byName = new Map<string, FlowScriptNodeInfo>();
	for (const node of nodes) {
		const info = buildNodeInfo(node);
		if (!byName.has(info.identifier)) byName.set(info.identifier, info);
	}
	return { byName, names: [...byName.keys()] };
}

let cachedNodes: INode[] | undefined;
let cachedIndex: FlowScriptIndex | undefined;

/** Memoized on catalog identity so it only rebuilds when the catalog prop changes. */
export function getFlowScriptIndex(
	nodes: INode[] | undefined,
): FlowScriptIndex {
	if (nodes === cachedNodes && cachedIndex) return cachedIndex;
	cachedIndex = buildFlowScriptIndex(nodes ?? []);
	cachedNodes = nodes;
	return cachedIndex;
}

function renderSignature(info: FlowScriptNodeInfo): string {
	if (info.args.length === 0) return `${info.identifier}()`;
	const params = info.args
		.map((arg) => `${arg.name}${arg.optional ? "?" : ""}: ${arg.typeString}`)
		.join(", ");
	return `${info.identifier}({ ${params} })`;
}

function nodeHoverMarkdown(info: FlowScriptNodeInfo): string {
	const lines: string[] = [];
	lines.push(`\`\`\`flowscript\n${renderSignature(info)}\n\`\`\``);
	const meta = [info.category, info.impure ? "impure" : "pure"]
		.filter(Boolean)
		.join(" · ");
	if (meta) lines.push(`_${meta}_`);
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
	lines.push(
		`\`${arg.name}${arg.optional ? "?" : ""}: ${arg.typeString}\` — argument of \`${info.identifier}\``,
	);
	if (arg.friendlyName && arg.friendlyName !== arg.name)
		lines.push(`**${arg.friendlyName}**`);
	if (arg.description) lines.push(arg.description);
	if (arg.enumValues && arg.enumValues.length > 0)
		lines.push(
			`Allowed: ${arg.enumValues.map((value) => `\`${value}\``).join(", ")}`,
		);
	if (arg.sensitive) lines.push("_Sensitive value_");
	return lines.join("\n\n");
}

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
		],
		surroundingPairs: [
			{ open: "{", close: "}" },
			{ open: "[", close: "]" },
			{ open: "(", close: ")" },
			{ open: '"', close: '"' },
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

	monaco.languages.setMonarchTokensProvider(FLOWSCRIPT_LANGUAGE_ID, {
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
		escapes: /\\(?:["\\/nrt]|u[0-9A-Fa-f]{4})/,
		tokenizer: {
			root: [
				// Anchor comments carry round-trip identity — highlight distinctly.
				[/\/\/@[a-z]:[^\n]*/, "comment.anchor"],
				[/\/\/.*$/, "comment"],
				// Decorators / annotations (@category, @secret, @readonly, …).
				[/@[A-Za-z_][\w]*/, "annotation"],
				// Declaration heads: keyword + declared name.
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
				// Property access on a return struct (`.value`, `.found`).
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
				[/[A-Za-z_$][\w$]*(?=\s*:)/, "variable.parameter"],
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
				[/[;,.]/, "delimiter"],
			],
			string: [
				[/[^\\"]+/, "string"],
				[/@escapes/, "string.escape"],
				[/\\./, "string.escape.invalid"],
				[/"/, { token: "string.quote", bracket: "@close", next: "@pop" }],
			],
			whitespace: [[/[ \t\r\n]+/, "white"]],
		},
	});
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

/** Blank out string/comment contents (preserving offsets) so bracket scans stay accurate. */
function maskLiterals(text: string): string {
	let out = "";
	let state: "code" | "string" | "comment" = "code";
	let i = 0;
	while (i < text.length) {
		const ch = text[i];
		if (state === "code") {
			if (ch === '"') {
				out += '"';
				state = "string";
			} else if (ch === "/" && text[i + 1] === "/") {
				out += "  ";
				i += 2;
				state = "comment";
				continue;
			} else {
				out += ch;
			}
		} else if (state === "string") {
			if (ch === "\\") {
				out += "  ";
				i += 2;
				continue;
			}
			if (ch === '"') {
				out += '"';
				state = "code";
			} else if (ch === "\n") {
				out += "\n";
				state = "code";
			} else {
				out += " ";
			}
		} else {
			if (ch === "\n") {
				out += "\n";
				state = "code";
			} else {
				out += " ";
			}
		}
		i++;
	}
	return out;
}

const IDENT_CHAR = /[A-Za-z0-9_$]/;

interface CallContext {
	callName: string;
	info?: FlowScriptNodeInfo;
	existingKeys: string[];
	mode: "key" | "value";
	activeArg?: string;
}

/**
 * Given masked text up to the cursor, determine whether the cursor sits inside a call's
 * argument object literal, which call it is, the keys already present, and whether we are
 * typing a key or a value.
 */
function analyzeContext(
	maskedBefore: string,
	index: FlowScriptIndex,
): CallContext | null {
	const stack: { ch: string; name?: string; open: number }[] = [];
	for (let i = 0; i < maskedBefore.length; i++) {
		const ch = maskedBefore[i];
		if (ch === "(" || ch === "{" || ch === "[") {
			let name: string | undefined;
			if (ch === "(") {
				let j = i - 1;
				while (j >= 0 && /\s/.test(maskedBefore[j])) j--;
				const end = j + 1;
				while (j >= 0 && IDENT_CHAR.test(maskedBefore[j])) j--;
				if (end > j + 1) name = maskedBefore.slice(j + 1, end);
			}
			stack.push({ ch, name, open: i });
		} else if (ch === ")" || ch === "}" || ch === "]") {
			stack.pop();
		}
	}

	const top = stack[stack.length - 1];
	if (!top || top.ch !== "{") return null;
	const parent = stack[stack.length - 2];
	if (!parent || parent.ch !== "(" || !parent.name) return null;

	const body = maskedBefore.slice(top.open + 1);
	const existingKeys: string[] = [];
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
	// `segment` is the text of the arg currently under the cursor.
	const active = /^\s*([A-Za-z_$][\w$]*)\s*:([\s\S]*)$/.exec(segment);
	if (active) {
		existingKeys.push(active[1]);
		return {
			callName: parent.name,
			info: index.byName.get(parent.name),
			existingKeys,
			mode: "value",
			activeArg: active[1],
		};
	}
	return {
		callName: parent.name,
		info: index.byName.get(parent.name),
		existingKeys,
		mode: "key",
	};
}

function offsetToPosition(
	text: string,
	offset: number,
): {
	lineNumber: number;
	column: number;
} {
	let line = 1;
	let lineStart = 0;
	for (let i = 0; i < offset; i++) {
		if (text[i] === "\n") {
			line++;
			lineStart = i + 1;
		}
	}
	return { lineNumber: line, column: offset - lineStart + 1 };
}

/**
 * Maps each variable/loop binding to the node whose result it holds, so `variable.` can offer
 * that node's output pins. Handles `const/let x = node(...)`, bare reassignments `x = node(...)`,
 * and loop bindings `for (const v of node(...))`.
 */
function collectVariableNodes(
	masked: string,
	index: FlowScriptIndex,
): Map<string, FlowScriptNodeInfo> {
	const map = new Map<string, FlowScriptNodeInfo>();
	const assign =
		/(?:^|[\n;{}])\s*(?:const\s+|let\s+)?([A-Za-z_$][\w$]*)\s*(?::[^=\n]+)?=\s*([A-Za-z_$][\w$]*)\s*\(/g;
	for (let m = assign.exec(masked); m; m = assign.exec(masked)) {
		const info = index.byName.get(m[2]);
		if (info) map.set(m[1], info);
	}
	const loop =
		/for\s*\(\s*(?:const|let)\s+([A-Za-z_$][\w$]*)\s+of\s+([A-Za-z_$][\w$]*)\s*\(/g;
	for (let m = loop.exec(masked); m; m = loop.exec(masked)) {
		const info = index.byName.get(m[2]);
		if (info) map.set(m[1], info);
	}
	return map;
}

interface DocumentSymbols {
	variables: Map<string, string | undefined>;
	functions: Set<string>;
	interfaces: Set<string>;
}

/** Scans the FlowScript document for its own declared variables, functions and interfaces. */
function collectDocumentSymbols(masked: string): DocumentSymbols {
	const variables = new Map<string, string | undefined>();
	const functions = new Set<string>();
	const interfaces = new Set<string>();

	const declRe =
		/(?:^|[\n;{}])\s*(?:const|let)\s+([A-Za-z_$][\w$]*)\s*(?::\s*([^=\n]+?))?\s*=/g;
	for (let m = declRe.exec(masked); m; m = declRe.exec(masked)) {
		variables.set(m[1], m[2]?.trim());
	}
	const loopRe = /for\s*\(\s*(?:const|let)\s+([A-Za-z_$][\w$]*)\s+(?:of|in)\b/g;
	for (let m = loopRe.exec(masked); m; m = loopRe.exec(masked)) {
		if (!variables.has(m[1])) variables.set(m[1], undefined);
	}
	const fnRe = /\b(?:function|event)\s+([A-Za-z_$][\w$]*)/g;
	for (let m = fnRe.exec(masked); m; m = fnRe.exec(masked)) functions.add(m[1]);
	const eventHeadRe = /(?:^|\n)\s*([A-Za-z_$][\w$]*)\s*\([^)]*\)\s*\{/g;
	for (let m = eventHeadRe.exec(masked); m; m = eventHeadRe.exec(masked)) {
		if (!KEYWORD_SET.has(m[1])) functions.add(m[1]);
	}
	const ifaceRe = /\b(?:interface|struct)\s+([A-Za-z_$][\w$]*)/g;
	for (let m = ifaceRe.exec(masked); m; m = ifaceRe.exec(masked))
		interfaces.add(m[1]);

	return { variables, functions, interfaces };
}

/**
 * Given masked text ending just before a member `.`, resolves the node whose outputs the member
 * accesses — the receiver may be a variable (`x.`) or a call result (`someNode({…}).`).
 */
function resolveMemberReceiver(
	beforeDot: string,
	maskedFull: string,
	index: FlowScriptIndex,
): FlowScriptNodeInfo | null {
	const receiver = beforeDot.replace(/\s+$/, "");
	if (receiver.endsWith(")")) {
		let depth = 0;
		let i = receiver.length - 1;
		for (; i >= 0; i--) {
			if (receiver[i] === ")") depth++;
			else if (receiver[i] === "(") {
				depth--;
				if (depth === 0) break;
			}
		}
		if (i < 0) return null;
		let j = i - 1;
		while (j >= 0 && /\s/.test(receiver[j])) j--;
		const end = j + 1;
		while (j >= 0 && IDENT_CHAR.test(receiver[j])) j--;
		const callName = receiver.slice(j + 1, end);
		return callName ? (index.byName.get(callName) ?? null) : null;
	}
	const idMatch = /([A-Za-z_$][\w$]*)$/.exec(receiver);
	if (idMatch)
		return collectVariableNodes(maskedFull, index).get(idMatch[1]) ?? null;
	return null;
}

function snippetPlaceholder(arg: FlowScriptArg, tabStop: number): string {
	if (arg.enumValues && arg.enumValues.length > 0) {
		return `"\${${tabStop}|${arg.enumValues.join(",")}|}"`;
	}
	return `\${${tabStop}:${arg.typeString}}`;
}

function buildCallSnippet(info: FlowScriptNodeInfo): string {
	if (info.args.length === 0) return `${info.identifier}()`;
	const params = info.args
		.map((arg, idx) => `${arg.name}: ${snippetPlaceholder(arg, idx + 1)}`)
		.join(", ");
	return `${info.identifier}({ ${params} })`;
}

function outputHoverMarkdown(
	info: FlowScriptNodeInfo,
	output: FlowScriptOutput,
): string {
	const lines = [
		`\`${output.name}: ${output.typeString}\` — output of \`${info.identifier}\``,
	];
	if (output.description) lines.push(output.description);
	return lines.join("\n\n");
}

/**
 * Registers completion, hover and signature-help providers for FlowScript, all backed by the
 * live catalog. Returns a single disposable that tears every provider down.
 */
export function registerFlowScriptProviders(
	monaco: Monaco,
	getCatalogNodes: () => INode[] | undefined,
): { dispose: () => void } {
	const completion = monaco.languages.registerCompletionItemProvider(
		FLOWSCRIPT_LANGUAGE_ID,
		{
			triggerCharacters: [".", "{", ",", " ", ":"],
			provideCompletionItems: (model, position) => {
				const index = getFlowScriptIndex(getCatalogNodes());
				const word = model.getWordUntilPosition(position);
				const range = {
					startLineNumber: position.lineNumber,
					endLineNumber: position.lineNumber,
					startColumn: word.startColumn,
					endColumn: word.endColumn,
				};

				const maskedFull = maskLiterals(model.getValue());
				const offset = model.getOffsetAt(position);

				// Dot notation on a variable OR a call result → offer the node's output pins.
				const beforeDot = maskedFull
					.slice(0, offset)
					.replace(/[A-Za-z_$][\w$]*$/, "")
					.replace(/\s*$/, "");
				if (beforeDot.endsWith(".")) {
					const source = resolveMemberReceiver(
						beforeDot.slice(0, -1),
						maskedFull,
						index,
					);
					if (source && source.outputs.length > 0) {
						return {
							suggestions: source.outputs.map((output) => ({
								label: output.name,
								kind: monaco.languages.CompletionItemKind.Property,
								detail: output.typeString,
								documentation: { value: outputHoverMarkdown(source, output) },
								insertText: output.name,
								range,
								sortText: `0_${output.name}`,
							})),
						};
					}
					return { suggestions: [] };
				}

				const context = analyzeContext(maskedFull.slice(0, offset), index);

				// Enum argument value → offer the allowed literals only.
				if (context?.mode === "value" && context.info && context.activeArg) {
					const arg = context.info.args.find(
						(candidate) => candidate.name === context.activeArg,
					);
					if (arg?.enumValues && arg.enumValues.length > 0) {
						return {
							suggestions: arg.enumValues.map((value) => ({
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
						suggestions: keyInfo.args
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

				// Default: document symbols, node calls, keywords, types and constants.
				const suggestions: unknown[] = [];
				const symbols = collectDocumentSymbols(maskedFull);
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
				for (const info of index.byName.values()) {
					suggestions.push({
						label: { label: info.identifier, description: info.friendlyName },
						kind: monaco.languages.CompletionItemKind.Function,
						detail: renderSignature(info),
						documentation: { value: nodeHoverMarkdown(info) },
						insertText: buildCallSnippet(info),
						insertTextRules:
							monaco.languages.CompletionItemInsertTextRule.InsertAsSnippet,
						filterText: `${info.identifier} ${info.friendlyName}`,
						range,
						sortText: `2_${info.identifier}`,
					});
				}
				for (const keyword of [...STORAGE_KEYWORDS, ...CONTROL_KEYWORDS]) {
					suggestions.push({
						label: keyword,
						kind: monaco.languages.CompletionItemKind.Keyword,
						insertText: keyword,
						range,
						sortText: `3_${keyword}`,
					});
				}
				for (const type of TYPE_KEYWORDS) {
					suggestions.push({
						label: type,
						kind: monaco.languages.CompletionItemKind.TypeParameter,
						insertText: type,
						range,
						sortText: `4_${type}`,
					});
				}
				for (const constant of CONSTANTS) {
					suggestions.push({
						label: constant,
						kind: monaco.languages.CompletionItemKind.Constant,
						insertText: constant,
						range,
						sortText: `4_${constant}`,
					});
				}
				return { suggestions: suggestions as never[] };
			},
		},
	);

	const hover = monaco.languages.registerHoverProvider(FLOWSCRIPT_LANGUAGE_ID, {
		provideHover: (model, position) => {
			const index = getFlowScriptIndex(getCatalogNodes());
			const word = model.getWordAtPosition(position);
			if (!word) return null;
			const range = {
				startLineNumber: position.lineNumber,
				endLineNumber: position.lineNumber,
				startColumn: word.startColumn,
				endColumn: word.endColumn,
			};

			const info = index.byName.get(word.word);
			if (info) {
				return { range, contents: [{ value: nodeHoverMarkdown(info) }] };
			}

			// Member access on a variable (`result.value`) → describe the source node's output pin.
			const lineBefore = model.getValueInRange({
				startLineNumber: position.lineNumber,
				startColumn: 1,
				endLineNumber: position.lineNumber,
				endColumn: word.startColumn,
			});
			const maskedLine = maskLiterals(lineBefore).replace(/\s*$/, "");
			if (maskedLine.endsWith(".")) {
				const source = resolveMemberReceiver(
					maskedLine.slice(0, -1),
					maskLiterals(model.getValue()),
					index,
				);
				const output = source?.outputs.find((out) => out.name === word.word);
				if (source && output) {
					return {
						range,
						contents: [{ value: outputHoverMarkdown(source, output) }],
					};
				}
			}

			const maskedBefore = maskLiterals(
				model.getValue().slice(0, model.getOffsetAt(position)),
			);
			const context = analyzeContext(maskedBefore, index);
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
		},
	});

	const signature = monaco.languages.registerSignatureHelpProvider(
		FLOWSCRIPT_LANGUAGE_ID,
		{
			signatureHelpTriggerCharacters: ["(", "{", ",", ":"],
			signatureHelpRetriggerCharacters: [",", ":"],
			provideSignatureHelp: (model, position) => {
				const index = getFlowScriptIndex(getCatalogNodes());
				const maskedBefore = maskLiterals(
					model.getValue().slice(0, model.getOffsetAt(position)),
				);
				const context = analyzeContext(maskedBefore, index);
				if (!context?.info || context.info.args.length === 0) return null;
				const info = context.info;

				let activeParameter = 0;
				if (context.mode === "value" && context.activeArg) {
					const idx = info.args.findIndex(
						(arg) => arg.name === context.activeArg,
					);
					if (idx >= 0) activeParameter = idx;
				} else {
					const present = new Set(context.existingKeys);
					const next = info.args.findIndex((arg) => !present.has(arg.name));
					activeParameter = next >= 0 ? next : info.args.length - 1;
				}

				return {
					value: {
						signatures: [
							{
								label: renderSignature(info),
								documentation: { value: info.description },
								parameters: info.args.map((arg) => ({
									label: `${arg.name}${arg.optional ? "?" : ""}: ${arg.typeString}`,
									documentation: { value: argHoverMarkdown(info, arg) },
								})),
							},
						],
						activeSignature: 0,
						activeParameter,
					},
					dispose: () => {},
				};
			},
		},
	);

	return {
		dispose: () => {
			completion.dispose();
			hover.dispose();
			signature.dispose();
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
	// Top-level event functions render as `name() {` with no keyword.
	const eventHead = /(^|\n)\s*([A-Za-z_$][\w$]*)\s*\([^)]*\)\s*\{/g;
	for (let m = eventHead.exec(text); m; m = eventHead.exec(text))
		names.add(m[2]);
	return names;
}

interface ArgLiteral {
	name: string;
	start: number;
	value: string;
	valueStart: number;
}

/** Parses a call's `{ key: value, … }` object into top-level key/value pairs with positions. */
function readArgs(masked: string, parenIndex: number): ArgLiteral[] | null {
	let i = parenIndex + 1;
	while (i < masked.length && /\s/.test(masked[i])) i++;
	if (masked[i] !== "{") return null;
	i++;
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

interface ValueType {
	group: TypeGroup;
	isArray: boolean;
	schemaTitle?: string;
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
});

function parseTypeAnnotation(text: string): ValueType | null {
	let base = text.trim();
	let isArray = false;
	if (base.endsWith("[]")) {
		isArray = true;
		base = base.slice(0, -2).trim();
	}
	const setMatch = /^Set<(.+)>$/.exec(base);
	if (setMatch) {
		isArray = true;
		base = setMatch[1].trim();
	}
	if (/^Map</.test(base)) return { group: "any", isArray: false };
	if (base.includes("|") || base.includes("("))
		return { group: "any", isArray };
	switch (base.toLowerCase()) {
		case "string":
			return { group: "string", isArray };
		case "int":
		case "float":
		case "number":
			return { group: "number", isArray };
		case "bool":
		case "boolean":
			return { group: "bool", isArray };
		case "date":
			return { group: "date", isArray };
		case "path":
		case "pathbuf":
			return { group: "path", isArray };
		case "byte":
		case "bytes":
			return { group: "bytes", isArray };
		case "any":
		case "generic":
		case "void":
			return { group: "any", isArray };
	}
	if (/^[A-Z]/.test(base))
		return { group: "struct", isArray, schemaTitle: base };
	return { group: "any", isArray };
}

/** Maps document variables to a resolved type from annotations or single-output node calls. */
function collectVariableTypes(
	masked: string,
	index: FlowScriptIndex,
): Map<string, ValueType | null> {
	const map = new Map<string, ValueType | null>();
	const annotated =
		/(?:^|[\n;{}])\s*(?:const|let)\s+([A-Za-z_$][\w$]*)\s*:\s*([^=\n]+?)\s*=/g;
	for (let m = annotated.exec(masked); m; m = annotated.exec(masked)) {
		map.set(m[1], parseTypeAnnotation(m[2]));
	}
	const assigned =
		/(?:^|[\n;{}])\s*(?:const|let)\s+([A-Za-z_$][\w$]*)\s*=\s*([A-Za-z_$][\w$]*)\s*\(/g;
	for (let m = assigned.exec(masked); m; m = assigned.exec(masked)) {
		if (map.has(m[1])) continue;
		const info = index.byName.get(m[2]);
		map.set(
			m[1],
			info && info.outputs.length === 1 ? pinValueType(info.outputs[0]) : null,
		);
	}
	const literal =
		/(?:^|[\n;{}])\s*(?:const|let)\s+([A-Za-z_$][\w$]*)\s*=\s*(?:("|\[|\{)|(-?\d)|\b(true|false)\b)/g;
	for (let m = literal.exec(masked); m; m = literal.exec(masked)) {
		if (map.has(m[1])) continue;
		if (m[2] === '"') map.set(m[1], { group: "string", isArray: false });
		else if (m[2] === "[") map.set(m[1], { group: "any", isArray: true });
		else if (m[2] === "{") map.set(m[1], { group: "struct", isArray: false });
		else if (m[3]) map.set(m[1], { group: "number", isArray: false });
		else map.set(m[1], { group: "bool", isArray: false });
	}
	return map;
}

function inferValueType(
	value: string,
	docVars: Map<string, ValueType | null>,
	varNodes: Map<string, FlowScriptNodeInfo>,
	index: FlowScriptIndex,
): ValueType | null {
	if (value === "" || value === "null")
		return value === "null" ? { group: "null", isArray: false } : null;
	if (value === "true" || value === "false")
		return { group: "bool", isArray: false };
	if (value.startsWith('"')) return { group: "string", isArray: false };
	if (/^-?\d+(\.\d+)?([eE][+-]?\d+)?$/.test(value))
		return { group: "number", isArray: false };
	if (value.startsWith("[")) return { group: "any", isArray: true };
	if (value.startsWith("{")) return { group: "struct", isArray: false };

	const memberMatch = /^([A-Za-z_$][\w$]*)\s*\.\s*([A-Za-z_$][\w$]*)$/.exec(
		value,
	);
	if (memberMatch) {
		const output = varNodes
			.get(memberMatch[1])
			?.outputs.find((out) => out.name === memberMatch[2]);
		return output ? pinValueType(output) : null;
	}
	const callMatch = /^([A-Za-z_$][\w$]*)\s*\(/.exec(value);
	if (callMatch) {
		const info = index.byName.get(callMatch[1]);
		if (!info || info.outputs.length === 0) return null;
		// Walk to the call's matching close paren, then see whether a `.output` follows.
		let depth = 0;
		let k = callMatch[0].length - 1;
		for (; k < value.length; k++) {
			if (value[k] === "(") depth++;
			else if (value[k] === ")") {
				depth--;
				if (depth === 0) {
					k++;
					break;
				}
			}
		}
		const afterCall = /^\s*\.\s*([A-Za-z_$][\w$]*)/.exec(value.slice(k));
		if (afterCall) {
			const out = info.outputs.find((o) => o.name === afterCall[1]);
			return out ? pinValueType(out) : null;
		}
		if (info.outputs.length === 1) return pinValueType(info.outputs[0]);
		return {
			group: "struct",
			isArray: false,
			multiOutput: {
				node: info.identifier,
				outputs: info.outputs.map((o) => o.name),
			},
		};
	}
	const bareMatch = /^[A-Za-z_$][\w$]*$/.exec(value);
	if (bareMatch) return docVars.get(value) ?? null;
	return null;
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

/**
 * Conservative client-side structural linter: unknown function calls, unknown/duplicate argument
 * keys, and best-effort type/schema mismatches (literal, variable or output value vs the pin's
 * expected type; struct schema titles only when both sides declare one). It only reports when both
 * sides are confidently known, skipping anything it cannot model to avoid false positives on valid
 * syntax; the authoritative parser runs server-side in the studio.
 */
export function computeFlowScriptDiagnostics(
	monaco: Monaco,
	text: string,
	nodes: INode[] | undefined,
): { markers: unknown[] } {
	const index = getFlowScriptIndex(nodes);
	if (index.names.length === 0) return { markers: [] };

	const masked = maskLiterals(text);
	const declared = collectDeclaredNames(text);
	const varNodes = collectVariableNodes(masked, index);
	const docVars = collectVariableTypes(masked, index);
	const raw: RawMarker[] = [];

	const callRe = /([A-Za-z_$][\w$]*)\s*\(/g;
	for (let match = callRe.exec(masked); match; match = callRe.exec(masked)) {
		const name = match[1];
		const nameStart = match.index;
		const prev = masked.slice(0, nameStart).trimEnd();
		if (prev.endsWith(".")) continue; // member access, not a node call
		if (KEYWORD_SET.has(name)) continue;

		const info = index.byName.get(name);
		if (!info) {
			if (!declared.has(name)) {
				raw.push({
					message: `Unknown function '${name}'. It is not a catalog node or a declared function.`,
					start: nameStart,
					end: nameStart + name.length,
					severity: "warning",
				});
			}
			continue;
		}

		const parenIndex = nameStart + match[0].length - 1;
		const args = readArgs(masked, parenIndex);
		if (!args) continue;
		const argsByName = new Map(info.args.map((arg) => [arg.name, arg]));
		const seen = new Set<string>();
		for (const arg of args) {
			const pin = argsByName.get(arg.name);
			if (!pin) {
				raw.push({
					message: `Unknown argument '${arg.name}' for '${name}'.`,
					start: arg.start,
					end: arg.start + arg.name.length,
					severity: "warning",
				});
				continue;
			}
			if (seen.has(arg.name)) {
				raw.push({
					message: `Duplicate argument '${arg.name}' for '${name}'.`,
					start: arg.start,
					end: arg.start + arg.name.length,
					severity: "warning",
				});
			}
			seen.add(arg.name);

			const actual = inferValueType(arg.value, docVars, varNodes, index);
			if (actual) {
				const reason = typeCompatibility(pinValueType(pin), actual);
				if (reason) {
					raw.push({
						message: describeMismatch(
							`Type mismatch for '${arg.name}' of '${name}'`,
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
		const lhsType = docVars.get(m[1]);
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
		const actual = inferValueType(rhs, docVars, varNodes, index);
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

	const markers = raw.map((marker) => {
		const start = offsetToPosition(text, marker.start);
		const end = offsetToPosition(text, marker.end);
		return {
			message: marker.message,
			severity:
				marker.severity === "error"
					? monaco.MarkerSeverity.Error
					: monaco.MarkerSeverity.Warning,
			startLineNumber: start.lineNumber,
			startColumn: start.column,
			endLineNumber: end.lineNumber,
			endColumn: end.column,
		};
	});
	return { markers };
}
