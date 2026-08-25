import { beforeAll, describe, expect, test } from "bun:test";
import type { Monaco } from "@monaco-editor/react";
import type { FlowScriptBoardScope } from "../../../lib/flow-modules";
import { loadFlowScriptNamesTable } from "../../../lib/flowscript/names";
import type { INode, IPin } from "../../../lib/schema/flow/node";
import {
	IPinType,
	IValueType,
	IVariableType,
} from "../../../lib/schema/flow/pin";
import {
	FLOWSCRIPT_MONARCH,
	catalogNamespaceRoots,
	computeFlowScriptDiagnostics,
	parseUseDeclarations,
	registerFlowScriptProviders,
} from "./flowscript-language";
import { tokenizeMonarch } from "./monarch-test-harness";

interface PinSpec {
	name: string;
	type: IVariableType;
	container?: IValueType;
	optional?: boolean;
	schema?: string;
	values?: string[];
}

function pins(
	inputs: PinSpec[],
	outputs: PinSpec[],
	impure = false,
): Record<string, IPin> {
	const out: Record<string, IPin> = {};
	let index = 0;
	const add = (spec: PinSpec, pinType: IPinType) => {
		const id = `${pinType}-${spec.name}`;
		out[id] = {
			id,
			name: spec.name,
			friendly_name: spec.name,
			description: "",
			pin_type: pinType,
			data_type: spec.type,
			value_type: spec.container ?? IValueType.Normal,
			index: index++,
			connected_to: [],
			depends_on: [],
			default_value: spec.optional ? [1] : null,
			schema: spec.schema ?? null,
			options: spec.values ? { valid_values: spec.values } : null,
		};
	};
	if (impure)
		add({ name: "exec_in", type: IVariableType.Execution }, IPinType.Input);
	for (const spec of inputs) add(spec, IPinType.Input);
	if (impure)
		add({ name: "exec_out", type: IVariableType.Execution }, IPinType.Output);
	for (const spec of outputs) add(spec, IPinType.Output);
	return out;
}

function node(
	name: string,
	inputs: PinSpec[],
	outputs: PinSpec[],
	extra: Partial<INode> = {},
): INode {
	return {
		id: name,
		name,
		friendly_name: name,
		description: `${name} node`,
		category: "Test",
		pins: pins(inputs, outputs, extra.impure === true),
		...extra,
	};
}

const RESPONSE_SCHEMA = JSON.stringify({
	title: "HttpResponse",
	type: "object",
	properties: { status: { type: "integer" }, body: { type: "string" } },
});

const catalog: INode[] = [
	node("events_generic", [], []),
	node("log_info", [{ name: "message", type: IVariableType.String }], [], {
		impure: true,
	}),
	node(
		"string_trim",
		[{ name: "string", type: IVariableType.String }],
		[{ name: "trimmed", type: IVariableType.String }],
	),
	node(
		"string_contains",
		[
			{ name: "string", type: IVariableType.String },
			{ name: "substring", type: IVariableType.String },
			{ name: "ignore_case", type: IVariableType.Boolean, optional: true },
		],
		[{ name: "contains", type: IVariableType.Boolean }],
	),
	node(
		"string_length",
		[{ name: "string", type: IVariableType.String }],
		[{ name: "length", type: IVariableType.Integer }],
	),
	node(
		"array_length",
		[
			{
				name: "array",
				type: IVariableType.Generic,
				container: IValueType.Array,
			},
		],
		[{ name: "length", type: IVariableType.Integer }],
	),
	node(
		"int_abs",
		[{ name: "integer", type: IVariableType.Integer }],
		[{ name: "result", type: IVariableType.Integer }],
	),
	node(
		"float_abs",
		[{ name: "float", type: IVariableType.Float }],
		[{ name: "result", type: IVariableType.Float }],
	),
	node(
		"utils_hash_md5",
		[{ name: "input", type: IVariableType.String }],
		[{ name: "hash", type: IVariableType.String }],
	),
	node(
		"http_fetch",
		[{ name: "url", type: IVariableType.String }],
		[{ name: "response", type: IVariableType.Struct, schema: RESPONSE_SCHEMA }],
		{ impure: true },
	),
	node(
		"http_response_to_text",
		[{ name: "response", type: IVariableType.Struct, schema: RESPONSE_SCHEMA }],
		[{ name: "text", type: IVariableType.String }],
		{ receiver: "response" },
	),
	node(
		"control_for_each",
		[
			{
				name: "array",
				type: IVariableType.Generic,
				container: IValueType.Array,
			},
		],
		[
			{ name: "value", type: IVariableType.Generic },
			{ name: "index", type: IVariableType.Integer },
		],
		{ impure: true },
	),
	// A third-party node that carries explicit names and no names.json row.
	node(
		"acme_wasm_lookup",
		[
			{ name: "key", type: IVariableType.String },
			{ name: "mode", type: IVariableType.String, values: ["fast", "exact"] },
		],
		[{ name: "value", type: IVariableType.String }],
		{ namespace: "acme.lookup", alias: "find", receiver: "" },
	),
	// A node with several execution outputs, the shape behind `bind { arm: { … } }` blocks.
	node(
		"stream_call",
		[{ name: "prompt", type: IVariableType.String }],
		[
			{ name: "exec_success", type: IVariableType.Execution },
			{ name: "exec_error", type: IVariableType.Execution },
			{ name: "response", type: IVariableType.String },
		],
	),
];

const diagnosticMonaco = {
	MarkerSeverity: { Error: 8, Warning: 4 },
} as unknown as Monaco;

interface TestPosition {
	lineNumber: number;
	column: number;
}

interface TestModel {
	uri: unknown;
	getValue: () => string;
	getOffsetAt: (position: TestPosition) => number;
	getWordUntilPosition: (position: TestPosition) => {
		word: string;
		startColumn: number;
		endColumn: number;
	};
}

interface TestCompletionItem {
	label: string | { label: string; description?: string };
	insertText?: string;
	filterText?: string;
	detail?: string;
	sortText?: string;
	additionalTextEdits?: TestTextEdit[];
	documentation?: { value: string };
}

interface TestRange {
	startLineNumber: number;
	startColumn: number;
	endLineNumber: number;
	endColumn: number;
}

interface TestTextEdit {
	range: TestRange;
	text: string;
}

interface TestMarker extends TestRange {
	message: string;
}

interface TestWorkspaceEdit {
	edits: { resource: unknown; textEdit: TestTextEdit; versionId?: number }[];
}

interface TestCodeAction {
	title: string;
	kind: string;
	edit: TestWorkspaceEdit;
}

interface TestSymbol {
	name: string;
	detail: string;
	kind: unknown;
	range: TestRange;
	selectionRange: TestRange;
	children: TestSymbol[];
}

type CodeActionCallback = (
	model: TestModel,
	range: TestRange,
) => { actions: TestCodeAction[] };
type SymbolsCallback = (model: TestModel) => TestSymbol[];
type FoldingCallback = (
	model: TestModel,
) => { start: number; end: number; kind?: unknown }[];
type InlayCallback = (
	model: TestModel,
	range?: TestRange,
) => { hints: { position: TestPosition; label: string; kind: unknown }[] };
type DefinitionCallback = (
	model: TestModel,
	position: TestPosition,
) => { uri: unknown; range: TestRange } | null;
type ReferencesCallback = (
	model: TestModel,
	position: TestPosition,
	context?: { includeDeclaration?: boolean },
) => { uri: unknown; range: TestRange }[];
interface TestSemanticProvider {
	getLegend: () => { tokenTypes: string[]; tokenModifiers: string[] };
	provideDocumentSemanticTokens: (model: TestModel) => { data: Uint32Array };
}
interface TestRenameProvider {
	resolveRenameLocation: (
		model: TestModel,
		position: TestPosition,
	) => { range: TestRange; text: string };
	provideRenameEdits: (
		model: TestModel,
		position: TestPosition,
		newName: string,
	) => TestWorkspaceEdit;
}

type CompletionCallback = (
	model: TestModel,
	position: TestPosition,
) => { suggestions: TestCompletionItem[] };

type HoverCallback = (
	model: TestModel & {
		getWordAtPosition: (
			position: TestPosition,
		) => { word: string; startColumn: number; endColumn: number } | null;
		getValueInRange: (range: {
			startLineNumber: number;
			startColumn: number;
			endLineNumber: number;
			endColumn: number;
		}) => string;
	},
	position: TestPosition,
) => { contents: { value: string }[] } | null;

type SignatureCallback = (
	model: TestModel,
	position: TestPosition,
) => {
	value: {
		signatures: { label: string; parameters: { label: string }[] }[];
		activeParameter: number;
	};
} | null;

function editorPosition(text: string): TestPosition {
	const lines = text.split("\n");
	return {
		lineNumber: lines.length,
		column: (lines.at(-1) ?? "").length + 1,
	};
}

function offsetAt(text: string, position: TestPosition): number {
	const lines = text.split("\n");
	return (
		lines
			.slice(0, position.lineNumber - 1)
			.reduce((sum, line) => sum + line.length + 1, 0) +
		position.column -
		1
	);
}

function registerTestProviders() {
	const completions: CompletionCallback[] = [];
	let hover: HoverCallback | undefined;
	let signature: SignatureCallback | undefined;
	let codeAction: CodeActionCallback | undefined;
	let symbols: SymbolsCallback | undefined;
	let folding: FoldingCallback | undefined;
	let inlay: InlayCallback | undefined;
	let definition: DefinitionCallback | undefined;
	let references: ReferencesCallback | undefined;
	let semantic: TestSemanticProvider | undefined;
	let rename: TestRenameProvider | undefined;
	let markers: TestMarker[] = [];
	const disposable = { dispose: () => undefined };
	const monaco = {
		editor: {
			getModelMarkers: () => markers,
		},
		languages: {
			CompletionItemKind: {
				Property: 1,
				EnumMember: 2,
				Field: 3,
				Variable: 4,
				Function: 5,
				Interface: 6,
				Keyword: 7,
				TypeParameter: 8,
				Constant: 9,
				Method: 10,
				Module: 11,
				Snippet: 12,
			},
			CompletionItemInsertTextRule: { InsertAsSnippet: 1 },
			SymbolKind: {
				Namespace: "namespace",
				Interface: "interface",
				Function: "function",
				Variable: "variable",
				Event: "event",
			},
			FoldingRangeKind: { Imports: "imports" },
			InlayHintKind: { Type: 1, Parameter: 2 },
			registerCompletionItemProvider: (
				_languageId: string,
				provider: { provideCompletionItems: CompletionCallback },
			) => {
				completions.push(provider.provideCompletionItems);
				return disposable;
			},
			registerHoverProvider: (
				_languageId: string,
				provider: { provideHover: HoverCallback },
			) => {
				hover = provider.provideHover;
				return disposable;
			},
			registerSignatureHelpProvider: (
				_languageId: string,
				provider: { provideSignatureHelp: SignatureCallback },
			) => {
				signature = provider.provideSignatureHelp;
				return disposable;
			},
			registerCodeActionProvider: (
				_languageId: string,
				provider: { provideCodeActions: CodeActionCallback },
			) => {
				codeAction = provider.provideCodeActions;
				return disposable;
			},
			registerDocumentSymbolProvider: (
				_languageId: string,
				provider: { provideDocumentSymbols: SymbolsCallback },
			) => {
				symbols = provider.provideDocumentSymbols;
				return disposable;
			},
			registerFoldingRangeProvider: (
				_languageId: string,
				provider: { provideFoldingRanges: FoldingCallback },
			) => {
				folding = provider.provideFoldingRanges;
				return disposable;
			},
			registerInlayHintsProvider: (
				_languageId: string,
				provider: { provideInlayHints: InlayCallback },
			) => {
				inlay = provider.provideInlayHints;
				return disposable;
			},
			registerDefinitionProvider: (
				_languageId: string,
				provider: { provideDefinition: DefinitionCallback },
			) => {
				definition = provider.provideDefinition;
				return disposable;
			},
			registerReferenceProvider: (
				_languageId: string,
				provider: { provideReferences: ReferencesCallback },
			) => {
				references = provider.provideReferences;
				return disposable;
			},
			registerDocumentSemanticTokensProvider: (
				_languageId: string,
				provider: TestSemanticProvider,
			) => {
				semantic = provider;
				return disposable;
			},
			registerRenameProvider: (
				_languageId: string,
				provider: TestRenameProvider,
			) => {
				rename = provider;
				return disposable;
			},
		},
	} as unknown as Monaco;

	const providers = registerFlowScriptProviders(monaco, () => catalog);
	const required = <T>(value: T | undefined, name: string): T => {
		if (!value) throw new Error(`${name} provider was not registered`);
		return value;
	};
	return {
		complete: () => required(completions[0], "Completion"),
		completeExtra: () => required(completions[1], "Snippet/auto-import"),
		hover: () => required(hover, "Hover"),
		signature: () => required(signature, "Signature"),
		codeAction: () => required(codeAction, "Code action"),
		symbols: () => required(symbols, "Document symbol"),
		folding: () => required(folding, "Folding"),
		inlay: () => required(inlay, "Inlay hint"),
		definition: () => required(definition, "Definition"),
		references: () => required(references, "Reference"),
		semantic: () => required(semantic, "Semantic tokens"),
		rename: () => required(rename, "Rename"),
		setMarkers: (next: TestMarker[]) => {
			markers = next;
		},
		dispose: providers.dispose,
	};
}

function testModel(text: string, position: TestPosition): TestModel {
	return {
		uri: {},
		getValue: () => text,
		getOffsetAt: (at) => offsetAt(text, at),
		getWordUntilPosition: () => ({
			word: "",
			startColumn: position.column,
			endColumn: position.column,
		}),
	};
}

function completionItems(text: string): TestCompletionItem[] {
	const providers = registerTestProviders();
	const position = editorPosition(text);
	const result = providers.complete()(testModel(text, position), position);
	providers.dispose();
	return result.suggestions;
}

function labelOf(item: TestCompletionItem): string {
	return typeof item.label === "string" ? item.label : item.label.label;
}

function completionLabels(text: string): string[] {
	return completionItems(text).map(labelOf);
}

function completionItem(
	text: string,
	label: string,
): TestCompletionItem | undefined {
	return completionItems(text).find((item) => labelOf(item) === label);
}

function hoverMarkdown(
	text: string,
	position: TestPosition,
): string | undefined {
	const providers = registerTestProviders();
	const hover = providers.hover();
	const lines = text.split("\n");
	const result = hover(
		{
			...testModel(text, position),
			getWordAtPosition: (at) => {
				const line = lines[at.lineNumber - 1] ?? "";
				let start = Math.max(0, at.column - 1);
				while (start > 0 && /[A-Za-z0-9_$]/.test(line[start - 1])) start--;
				let end = Math.max(0, at.column - 1);
				while (end < line.length && /[A-Za-z0-9_$]/.test(line[end])) end++;
				return end > start
					? {
							word: line.slice(start, end),
							startColumn: start + 1,
							endColumn: end + 1,
						}
					: null;
			},
			getValueInRange: (range) => {
				if (range.startLineNumber !== range.endLineNumber) return "";
				return (lines[range.startLineNumber - 1] ?? "").slice(
					range.startColumn - 1,
					Math.min(
						range.endColumn - 1,
						lines[range.startLineNumber - 1].length,
					),
				);
			},
		},
		position,
	);
	providers.dispose();
	return result?.contents[0]?.value;
}

/** Hover over the first occurrence of `word` in `text`. */
function hoverAt(text: string, word: string): string | undefined {
	const offset = text.indexOf(word);
	if (offset < 0) throw new Error(`'${word}' not found in text`);
	const before = text.slice(0, offset);
	const lineNumber = before.split("\n").length;
	const column = offset - before.lastIndexOf("\n");
	return hoverMarkdown(text, { lineNumber, column: column + 1 });
}

function signatureHelp(text: string) {
	const providers = registerTestProviders();
	const position = editorPosition(text);
	const result = providers.signature()(testModel(text, position), position);
	providers.dispose();
	return result?.value;
}

function diagnosticMessages(
	text: string,
	board?: FlowScriptBoardScope,
): string[] {
	const { markers } = computeFlowScriptDiagnostics(
		diagnosticMonaco,
		text,
		catalog,
		board,
	);
	return (markers as { message: string }[]).map((marker) => marker.message);
}

function tokens(line: string): string[] {
	return tokenizeMonarch(FLOWSCRIPT_MONARCH as never, line)
		.filter((token) => token.type !== "white")
		.map((token) => `${token.text}=${token.type}`);
}

beforeAll(async () => {
	await loadFlowScriptNamesTable();
});

describe("FlowScript tokenizer", () => {
	test("colours method calls as functions and plain members as properties", () => {
		expect(tokens("s.trim().value")).toEqual([
			"s=identifier",
			".=delimiter",
			"trim=entity.name.function",
			"()=delimiter.parenthesis",
			".=delimiter",
			"value=property",
		]);
	});

	test("colours namespace paths and the `::` separator", () => {
		expect(tokens("hash::md5({ input: x })")).toEqual([
			"hash=entity.name.namespace",
			"::=delimiter.path",
			"md5=entity.name.function",
			"(=delimiter.parenthesis",
			"{=delimiter.curly",
			"input=variable.parameter",
			":=",
			"x=identifier",
			"}=delimiter.curly",
			")=delimiter.parenthesis",
		]);
		expect(tokens("ai::ml::model::read({ path })").slice(0, 6)).toEqual([
			"ai=entity.name.namespace",
			"::=delimiter.path",
			"ml=entity.name.namespace",
			"::=delimiter.path",
			"model=entity.name.namespace",
			"::=delimiter.path",
		]);
	});

	test("tokenizes template literals with nested expressions", () => {
		expect(tokens("`Topic ${label} ${a.b().c[0]}!`")).toEqual([
			"`=string.quote",
			"Topic =string",
			"${=delimiter.template",
			"label=identifier",
			"}=delimiter.template",
			" =string",
			"${=delimiter.template",
			"a=identifier",
			".=delimiter",
			"b=entity.name.function",
			"()=delimiter.parenthesis",
			".=delimiter",
			"c=property",
			"[=delimiter.square",
			"0=number",
			"]=delimiter.square",
			"}=delimiter.template",
			"!=string",
			"`=string.quote",
		]);
	});

	test("tokenizes use lines, single quotes, @parallel and compound assignment", () => {
		expect(tokens("use string::*, ui::{ setElementText }").slice(0, 5)).toEqual(
			[
				"use=keyword",
				"string=entity.name.namespace",
				"::=delimiter.path",
				"*=operator",
				",=delimiter",
			],
		);
		expect(tokens("use a::b as x")).toContain("as=keyword.control");
		expect(tokens("let s = 'x'")).toEqual([
			"let=keyword",
			"s=variable",
			"==operator",
			"'=string.quote",
			"x=string",
			"'=string.quote",
		]);
		expect(tokens("@parallel for (const it of xs) { n += 1 }")).toContain(
			"@parallel=annotation",
		);
		expect(tokens("n += 1")).toContain("+==operator");
	});
});

describe("parseUseDeclarations", () => {
	test("parses every use-tree form with offsets", () => {
		const text = `use ai::ml
use ai::ml::*
use data::atlassian::jira as jira
use ui::{ setElementText, navigateTo }
use string::*, array::*

eventsSimple onLoad() {
	use nested::inside
}`;
		const uses = parseUseDeclarations(text);
		expect(uses.map((use) => use.kind)).toEqual([
			"namespace",
			"glob",
			"alias",
			"members",
			"glob",
			"glob",
		]);
		expect(uses[0].path).toEqual(["ai", "ml"]);
		expect(uses[2]).toMatchObject({
			path: ["data", "atlassian", "jira"],
			alias: "jira",
		});
		expect(uses[3]).toMatchObject({
			path: ["ui"],
			members: ["setElementText", "navigateTo"],
		});
		expect(uses[5].path).toEqual(["array"]);
		expect(text.slice(uses[5].start, uses[5].end)).toBe("array::*");
		expect(text.slice(uses[3].start, uses[3].end)).toBe(
			"ui::{ setElementText, navigateTo }",
		);
	});

	test("reports malformed trees and ignores strings, comments and identifiers named use", () => {
		const uses = parseUseDeclarations(
			'use string::{}\nuse a::* as b\nconst usesThing = "use x::y"\n// use ignored::*\nlet use = 1',
		);
		expect(uses).toHaveLength(2);
		expect(uses[0]).toMatchObject({ kind: "invalid" });
		expect(uses[1]).toMatchObject({ kind: "invalid" });
		expect(parseUseDeclarations("use string::*;")).toHaveLength(1);
	});
});

describe("FlowScript completions", () => {
	test("lists namespace members after `string::` with qualified-form snippets", () => {
		const items = completionItems("eventsSimple onLoad() {\n\tstring::");
		const labels = items.map(labelOf);
		expect(labels).toContain("trim");
		expect(labels).toContain("contains");
		expect(labels).not.toContain("md5");
		expect(items.find((item) => labelOf(item) === "trim")?.insertText).toBe(
			"trim({ string: ${1:string} })",
		);
	});

	test("resolves `use` aliases and nested namespaces in path completion", () => {
		expect(completionLabels("use acme::lookup as lk\n\tlk::")).toContain(
			"find",
		);
		expect(completionLabels("acme::")).toContain("lookup");
		expect(completionItem("acme::", "lookup")?.insertText).toBe("lookup::");
	});

	test("offers string methods after a string-typed receiver", () => {
		const items = completionItems('const s = "  hi "\nconst t = s.');
		const labels = items.map(labelOf);
		expect(labels).toContain("trim");
		expect(labels).toContain("contains");
		expect(labels).not.toContain("abs");
		expect(labels).toContain("md5");
		const contains = items.find((item) => labelOf(item) === "contains");
		expect(contains?.insertText).toBe(
			"contains({ substring: ${1:string}, ignoreCase: ${2:bool} })",
		);
		expect(completionItem('const s = "x"\ns.', "trim")?.insertText).toBe(
			"trim()",
		);
	});

	test("picks int methods for integer literals and call results", () => {
		expect(completionLabels("const n = (5).")).toContain("abs");
		expect(completionLabels("const n = (5).")).not.toContain("trim");
		expect(completionLabels('const n = "x".length().')).toContain("abs");
	});

	test("lists struct fields, outputs and titled-struct methods together", () => {
		const labels = completionLabels(
			'const r = http::fetch({ url: "u" })\nconst t = r.',
		);
		expect(labels).toContain("status");
		expect(labels).toContain("body");
		expect(labels).toContain("toText");
		expect(labels).not.toContain("trim");
	});

	test("offers every method grouped by class when the receiver type is unknown", () => {
		const items = completionItems("const t = unknownThing.");
		const trim = items.find((item) => labelOf(item) === "trim");
		expect(trim).toBeDefined();
		expect(items.map(labelOf)).toContain("abs");
		expect(
			typeof trim?.label === "object" ? trim.label.description : "",
		).toContain("string");
	});

	test("opens bare members via `use string::*` and keeps qualified spellings otherwise", () => {
		const opened = completionItems("use string::*\n\t");
		const trim = opened.find((item) => labelOf(item) === "trim");
		expect(trim?.insertText).toBe("trim({ string: ${1:string} })");
		expect(opened.map(labelOf)).toContain("hash::md5");
		expect(opened.map(labelOf)).not.toContain("string::trim");

		const closed = completionItems("eventsSimple onLoad() {\n\t");
		const qualified = closed.find((item) => labelOf(item) === "string::trim");
		expect(qualified?.insertText).toBe("string::trim({ string: ${1:string} })");
		expect(qualified?.filterText).toContain("stringTrim");
		expect(closed.map(labelOf)).toContain("string");
		expect(closed.map(labelOf)).not.toContain("trim");
	});

	test("skips the receiver and positional arguments when completing named keys", () => {
		expect(completionLabels('const s = "a"\ns.contains({ ')).toEqual([
			"substring",
			"ignoreCase",
		]);
		expect(completionLabels('string::contains("a", { ')).toEqual([
			"substring",
			"ignoreCase",
		]);
		expect(completionLabels('string::contains("a", "b", { ')).toEqual([
			"ignoreCase",
		]);
	});

	test("offers enum values for a positional argument", () => {
		expect(completionLabels('acme::lookup::find("k", ')).toEqual([
			'"fast"',
			'"exact"',
		]);
	});

	test("does not treat destructured or loop bindings as unknown", () => {
		const labels = completionLabels(
			'const { text, hash: h } = hash::md5({ input: "x" })\nfor (const [i, item] of items) {\n\t',
		);
		expect(labels).toContain("h");
		expect(labels).toContain("item");
		expect(labels).toContain("i");
	});
});

describe("FlowScript hover and signature help", () => {
	test("hover resolves flat, qualified and method spellings to the same node", () => {
		const flat = hoverAt('stringTrim({ string: "x" })', "stringTrim");
		const qualified = hoverAt('string::trim({ string: "x" })', "trim");
		const method = hoverAt('const s = "x"\nconst t = s.trim()', "trim()");
		for (const markdown of [flat, qualified, method]) {
			expect(markdown).toContain("string::trim({ string: string })");
			expect(markdown).toContain("legacy `stringTrim(…)`");
			expect(markdown).toContain("x.trim()");
		}
	});

	test("hover describes namespaces and opened members", () => {
		expect(hoverAt("hash::md5({})", "hash")).toContain("use hash::*");
		expect(
			hoverAt("use string::*\nconst t = trim({ string: s })", "trim("),
		).toContain("string::trim");
	});

	test("signature help hides the receiver in method form and tracks positional args", () => {
		const named = signatureHelp('const s = "x"\ns.contains({ ');
		expect(named?.signatures[0].label).toBe(
			"string.contains({ substring: string, ignoreCase?: bool })",
		);
		expect(named?.activeParameter).toBe(0);

		const positional = signatureHelp('string::contains("a", ');
		expect(positional?.signatures[0].label).toContain("string::contains(");
		expect(positional?.activeParameter).toBe(1);
	});
});

describe("FlowScript diagnostics", () => {
	test("legacy flat calls and every new spelling stay clean", () => {
		const text = `use string::*
use acme::lookup as lk

eventsGeneric onLoad(payload: Struct) {
	const s = "  hi  "
	const t = stringTrim({ string: s })
	const u = string::trim({ string: s })
	const v = s.trim()
	const w = "lit".contains("?", { ignoreCase: true })
	const x = trim({ string: s })
	const y = hash::md5({ input: s }).hash
	const z = lk::find("k", "fast")
	const { hash: h } = utilsHashMd5({ input: s })
	const n = (5).abs()
	const msg = \`Hello \${s.trim()} and \${h}\`
	let count = 0
	count += 1
	@parallel for (const [i, item] of items) {
		logInfo({ message: item })
	}
	while (!done) { log::info({ message: msg }); }
}`;
		expect(diagnosticMessages(text)).toEqual([]);
	});

	test("flags unknown namespaces in use lines and unknown members", () => {
		const messages = diagnosticMessages(
			"use nope::*\nuse string::{ trim, nothing }\n",
		);
		expect(messages).toHaveLength(2);
		expect(messages[0]).toContain("Unknown namespace 'nope'");
		expect(messages[1]).toContain(
			"'nothing' is not a member of namespace 'string'",
		);
	});

	test("flags unknown qualified calls and suggests the right spelling", () => {
		const messages = diagnosticMessages(
			'array::trim({ string: "x" })\nnope::thing()\ntrim({ string: "x" })',
		);
		expect(messages).toHaveLength(3);
		expect(messages[0]).toContain(
			"'trim' is not a member of namespace 'array'",
		);
		expect(messages[0]).toContain("`string::trim(…)`");
		expect(messages[1]).toContain("Namespace 'nope' is not in the catalog");
		expect(messages[2]).toContain("Unknown function 'trim'");
		expect(messages[2]).toContain("`string::trim(…)`");
	});

	test("validates method calls against the node minus the receiver pin", () => {
		const messages = diagnosticMessages(
			'const s = "x"\nconst a = s.contains({ substring: "?", bogus: 1 })\nconst b = s.contains({ string: s, substring: "?" })\nconst c = s.abs()',
		);
		expect(messages).toHaveLength(3);
		expect(messages[0]).toContain("Unknown argument 'bogus'");
		expect(messages[1]).toContain("already bound by the receiver");
		expect(messages[2]).toContain("Unknown method 'abs' on string");
		expect(messages[2]).toContain("`int::abs(…)`");
	});

	test("flags positional overflow and positional type mismatches", () => {
		const messages = diagnosticMessages(
			'const s = "x"\ns.contains("a", true, 3)\nstring::contains(1, "b")\nstring::trim("a", { string: "b" })',
		);
		expect(messages).toHaveLength(3);
		expect(messages[0]).toContain(
			"Too many positional arguments for '.contains()'",
		);
		expect(messages[1]).toContain(
			"Type mismatch for 'string' of 'string::contains'",
		);
		expect(messages[2]).toContain("already bound positionally");
	});

	test("does not flag calls inside template expressions or user UFCS methods", () => {
		const text = `function shout(s: string): (out: string) { return s }
eventsSimple onLoad() {
	const s = "x"
	const a = s.shout()
	const b = \`\${mystery()} and \${s.trim()}\`
}`;
		expect(diagnosticMessages(text)).toEqual([]);
	});
});

describe("FlowScript modules", () => {
	const board: FlowScriptBoardScope = {
		modules: ["checkout", "checkout::payments"],
		functionsByModule: {
			"": ["rootHelper"],
			checkout: ["total"],
			"checkout::payments": ["capture"],
		},
	};

	test("colours `module` as a keyword in header position only", () => {
		expect(tokens("module checkout {")).toEqual([
			"module=keyword",
			"checkout=entity.name.namespace",
			"{=delimiter.curly",
		]);
		// The same word elsewhere is an ordinary identifier — nothing else may be recoloured.
		expect(tokens("logInfo({ message: module })")).toContain(
			"module=identifier",
		);
		expect(tokens("const module = 1")).toContain("module=variable");
	});

	test("keeps module blocks and cross-file calls out of the diagnostics", () => {
		const text = `module checkout {
	function total(): (out: int) {
		return 1
	}

	eventsGeneric onLoad(payload: Struct) {
		const a = checkout::payments::capture("x")
		const b = rootHelper()
		const c = total()
	}
}`;
		expect(diagnosticMessages(text, board)).toEqual([]);
	});

	test("still flags a path that is neither a catalog namespace nor a board module", () => {
		const messages = diagnosticMessages("nope::thing()", board);
		expect(messages).toHaveLength(1);
		expect(messages[0]).toContain("Namespace 'nope' is not in the catalog");
	});

	test("without board context a module path is still an unknown namespace", () => {
		const messages = diagnosticMessages("checkout::payments::capture('x')");
		expect(messages).toHaveLength(1);
		expect(messages[0]).toContain(
			"Namespace 'checkout::payments' is not in the catalog",
		);
	});

	test("outlines module blocks with their sections nested inside", () => {
		const text = `module checkout {
	const fee = 1

	interface Cart {
		total: int;
	}

	function total(): (out: int) {
		return 1
	}

	eventsGeneric onLoad(payload: Struct) {
	}
}

function rootHelper(): (out: int) {
	return 2
}`;
		const symbols = documentSymbols(text);
		const module = symbols.find((symbol) => symbol.name === "checkout");
		expect(module?.kind).toBe("namespace");
		expect(module?.children.map((child) => child.name)).toEqual([
			"fee",
			"Cart",
			"total",
			"onLoad",
		]);
		// A module's sections are top level for the outline, not nested handlers.
		expect(
			module?.children.find((child) => child.name === "onLoad")?.kind,
		).toBe("event");
		expect(symbols.map((symbol) => symbol.name)).toContain("rootHelper");
	});

	test("folds a module block", () => {
		const text = "module checkout {\n\tconst fee = 1\n}\n";
		const providers = registerTestProviders();
		const ranges = providers.folding()(testModel(text, editorPosition(text)));
		providers.dispose();
		expect(ranges.some((range) => range.start === 1 && range.end === 2)).toBe(
			true,
		);
	});

	test("exposes the catalog namespace roots module names must avoid", () => {
		const roots = catalogNamespaceRoots(catalog);
		expect(roots).toContain("string");
		expect(roots).toContain("hash");
		// Roots only — nested namespaces would never collide with a module's first segment.
		expect(roots.every((root) => !root.includes("::"))).toBe(true);
	});
});

describe("FlowScript detached blocks", () => {
	test("colours `detached` as a keyword in header position only", () => {
		expect(tokens("detached {")).toEqual([
			"detached=keyword",
			"{=delimiter.curly",
		]);
		// The header is the bare word before the brace; anything else keeps its ordinary colour.
		expect(tokens("const detached = 1")).toContain("detached=variable");
		expect(tokens("logInfo({ message: detached })")).toContain(
			"detached=identifier",
		);
		expect(tokens("logInfo({ detached: 1 })")).toContain(
			"detached=variable.parameter",
		);
		// `detached(…) { }` is still an event named `detached`, not a detached block.
		expect(tokens("detached(payload: Struct) {")).toContain(
			"detached=entity.name.function",
		);
	});

	test("keeps a detached block out of the diagnostics", () => {
		const text = `detached {
	logInfo({ message: "keep me" })
}`;
		expect(diagnosticMessages(text)).toEqual([]);
	});

	test("treats a detached block's contents as statements, not declarations", () => {
		const text = `const shipped = 1

detached {
	const fee = 2

	interface Cart {
		total: int;
	}
}`;
		const semantic = decodeSemanticTokens(text);
		const at = (word: string, line: number) =>
			semantic.find((token) => token.word === word && token.line === line);
		expect(at("shipped", 1)?.type).toBe("variable");
		// Unlike `module`, a detached block is a chain of statements — its `const` is a local.
		expect(at("fee", 4)?.type).toBe("local");
		// Only the real top-level declaration is outlined: the block's `interface` is a statement.
		expect(documentSymbols(text).map((symbol) => symbol.name)).toEqual([
			"shipped",
		]);
	});

	test("folds a detached block", () => {
		const text = 'detached {\n\tlogInfo({ message: "x" })\n}\n';
		const providers = registerTestProviders();
		const ranges = providers.folding()(testModel(text, editorPosition(text)));
		providers.dispose();
		expect(ranges.some((range) => range.start === 1 && range.end === 2)).toBe(
			true,
		);
	});
});

describe("canonical FlowScript event headers", () => {
	test("treats the second identifier as the declared event alias", () => {
		const messages = diagnosticMessages(
			"eventsGeneric wikiExplorerLoad(payload: Struct) {\n\tlogUnknown()\n}",
		);

		expect(messages).toHaveLength(1);
		expect(messages[0]).toContain("Unknown function 'logUnknown'");
		expect(messages[0]).not.toContain("wikiExplorerLoad");
	});

	test("offers the event alias as a document function completion", () => {
		const labels = completionLabels(
			"eventsGeneric wikiExplorerLoad(payload: Struct) {\n}",
		);

		expect(labels).toContain("wikiExplorerLoad");
	});
});

describe("FlowScript function cache editor support", () => {
	test("offers bare and configured cache decorator snippets", () => {
		const items = completionItems("@ca");
		const labels = items.map(labelOf);

		expect(labels).toEqual(["@cache", "@cache({ … })", "@parallel"]);
		expect(items[1]?.insertText).toBe(
			'@cache({ namespace: "${1:global}", ttlSeconds: ${2:300}, scope: "${3|app,user|}" })',
		);
	});

	test("offers only missing cache settings inside the decorator", () => {
		const items = completionItems('@cache({ namespace: "pricing", ');
		const labels = items.map(labelOf);

		expect(labels).toEqual(["ttlSeconds", "scope"]);
		expect(labels).not.toContain("events::generic");
		expect(items[0]?.insertText).toBe("ttlSeconds: ${1:300}");
	});

	test("offers canonical app and user scope values", () => {
		expect(completionLabels('@cache({ scope: "')).toEqual(['"app"', '"user"']);
	});

	test("documents cache semantics on decorator hover", () => {
		const markdown = hoverMarkdown("@cache", { lineNumber: 1, column: 3 });

		expect(markdown).toContain("skips the entire function body");
		expect(markdown).toContain('`"global"` namespace');
		expect(markdown).toContain("300-second lifetime");
		expect(markdown).toContain("`ttlSeconds: 0`");
	});

	test("does not lint the cache decorator as an unknown function", () => {
		const text = `@cache({ namespace: "pricing" })
function calculatePricing(): void {
}`;
		expect(diagnosticMessages(text)).toHaveLength(0);
	});

	test("keeps catalog completions available outside cache settings", () => {
		const item = completionItem(
			"function calculatePricing() {\n\t",
			"events::generic",
		);
		expect(item).toBeDefined();
		expect(item?.filterText).toContain("eventsGeneric");
	});
});

// ---------------------------------------------------------------------------
// Feature-provider helpers
// ---------------------------------------------------------------------------

function positionAt(text: string, offset: number): TestPosition {
	const before = text.slice(0, offset);
	const lines = before.split("\n");
	return { lineNumber: lines.length, column: (lines.at(-1) ?? "").length + 1 };
}

/** Position of the first occurrence of `target`, offset by `skip` occurrences. */
function positionOfWord(text: string, target: string, skip = 0): TestPosition {
	let offset = -1;
	for (let i = 0; i <= skip; i++) {
		offset = text.indexOf(target, offset + 1);
		if (offset < 0) throw new Error(`'${target}' not found in text`);
	}
	return positionAt(text, offset + 1);
}

function applyTestEdit(text: string, edit: TestTextEdit): string {
	const start = offsetAt(text, {
		lineNumber: edit.range.startLineNumber,
		column: edit.range.startColumn,
	});
	const end = offsetAt(text, {
		lineNumber: edit.range.endLineNumber,
		column: edit.range.endColumn,
	});
	return text.slice(0, start) + edit.text + text.slice(end);
}

function singleEdit(action: TestCodeAction): TestTextEdit {
	expect(action.edit.edits).toHaveLength(1);
	return action.edit.edits[0].textEdit;
}

function codeActionsFor(
	text: string,
	target: string,
	message: string,
): TestCodeAction[] {
	const providers = registerTestProviders();
	const offset = text.indexOf(target);
	if (offset < 0) throw new Error(`'${target}' not found in text`);
	const start = positionAt(text, offset);
	const end = positionAt(text, offset + target.length);
	providers.setMarkers([
		{
			message,
			startLineNumber: start.lineNumber,
			startColumn: start.column,
			endLineNumber: end.lineNumber,
			endColumn: end.column,
		},
	]);
	const result = providers.codeAction()(testModel(text, end), {
		startLineNumber: 1,
		startColumn: 1,
		endLineNumber: text.split("\n").length,
		endColumn: 10_000,
	});
	providers.dispose();
	return result.actions;
}

function extraCompletionItems(text: string): TestCompletionItem[] {
	const providers = registerTestProviders();
	const position = editorPosition(text);
	const result = providers.completeExtra()(testModel(text, position), position);
	providers.dispose();
	return result.suggestions;
}

function extraCompletionItem(
	text: string,
	label: string,
): TestCompletionItem | undefined {
	return extraCompletionItems(text).find((item) => labelOf(item) === label);
}

function documentSymbols(text: string): TestSymbol[] {
	const providers = registerTestProviders();
	const result = providers.symbols()(testModel(text, editorPosition(text)));
	providers.dispose();
	return result;
}

function decodeSemanticTokens(
	text: string,
): { word: string; type: string; line: number; declaration: boolean }[] {
	const providers = registerTestProviders();
	const provider = providers.semantic();
	const legend = provider.getLegend();
	const result = provider.provideDocumentSemanticTokens(
		testModel(text, { lineNumber: 1, column: 1 }),
	);
	providers.dispose();
	const lines = text.split("\n");
	const tokens: {
		word: string;
		type: string;
		line: number;
		declaration: boolean;
	}[] = [];
	let line = 0;
	let char = 0;
	for (let i = 0; i < result.data.length; i += 5) {
		line += result.data[i];
		char =
			result.data[i] === 0 ? char + result.data[i + 1] : result.data[i + 1];
		tokens.push({
			word: (lines[line] ?? "").slice(char, char + result.data[i + 2]),
			type: legend.tokenTypes[result.data[i + 3]],
			line: line + 1,
			declaration: (result.data[i + 4] & 1) === 1,
		});
	}
	return tokens;
}

describe("FlowScript quick fixes", () => {
	test("offers did-you-mean replacements from backtick candidates", () => {
		const actions = codeActionsFor(
			'trm({ string: "x" })',
			"trm",
			"Unknown function 'trm'. It is not a catalog node or a declared function. Did you mean `string::trim(…)`?",
		);
		const replace = actions.find(
			(action) => action.title === "Replace with 'string::trim'",
		);
		expect(replace).toBeDefined();
		const edit = singleEdit(replace as TestCodeAction);
		expect(edit.text).toBe("string::trim");
		expect(applyTestEdit('trm({ string: "x" })', edit)).toBe(
			'string::trim({ string: "x" })',
		);
	});

	test("offers one rewrite per ambiguity candidate", () => {
		const text = "length({ string: s })";
		const actions = codeActionsFor(
			text,
			"length",
			"'length' is ambiguous: `string::length`, `array::length`. Write the qualified name.",
		);
		expect(actions.map((action) => action.title)).toEqual([
			"Replace with 'string::length'",
			"Replace with 'array::length'",
		]);
		expect(applyTestEdit(text, singleEdit(actions[0]))).toBe(
			"string::length({ string: s })",
		);
	});

	test("inserts the use line a backend diagnostic names", () => {
		const text = 'trim({ string: "x" })';
		const actions = codeActionsFor(
			text,
			"trim",
			"FlowScript call `trim` does not match a catalog declaration; did you mean `string::trim` (or add `use string::*` to call it bare)?",
		);
		const addUse = actions.find(
			(action) => action.title === "Add 'use string::*'",
		);
		expect(addUse).toBeDefined();
		expect(applyTestEdit(text, singleEdit(addUse as TestCodeAction))).toBe(
			'use string::*\n\ntrim({ string: "x" })',
		);
		expect(
			actions.some((action) => action.title === "Replace with 'string::trim'"),
		).toBe(true);
	});

	test("stubs missing required inputs with typed placeholders", () => {
		const text = "logInfo({})";
		const actions = codeActionsFor(
			text,
			"logInfo",
			"node `logInfo` is missing required inputs: message",
		);
		const fix = actions.find(
			(action) => action.title === "Add missing input: message",
		);
		expect(fix).toBeDefined();
		expect(applyTestEdit(text, singleEdit(fix as TestCodeAction))).toBe(
			'logInfo({ message: "" })',
		);
	});

	test("removes an argument that duplicates the bound receiver", () => {
		const text = 'const s = "x"\nconst t = s.trim({ string: s })';
		const actions = codeActionsFor(
			text,
			"string: s",
			"Argument 'string' is already bound by the receiver of '.trim()'.",
		);
		const fix = actions.find(
			(action) => action.title === "Remove duplicate argument 'string'",
		);
		expect(fix).toBeDefined();
		expect(applyTestEdit(text, singleEdit(fix as TestCodeAction))).toBe(
			'const s = "x"\nconst t = s.trim({  })',
		);
	});

	test("never rewrites a method member into a qualified path", () => {
		const actions = codeActionsFor(
			'const s = "x"\nconst n = s.abs()',
			"abs",
			"Unknown method 'abs' on string. Did you mean `int::abs(…)`?",
		);
		expect(
			actions.some((action) => action.title.startsWith("Replace with")),
		).toBe(false);
	});
});

describe("FlowScript auto-import completions", () => {
	test("offers members of unopened namespaces with a use-line edit", () => {
		const text = "eventsSimple onLoad() {\n\t";
		const item = extraCompletionItem(text, "trim");
		expect(item).toBeDefined();
		expect(item?.detail).toContain("use string::*");
		expect(item?.insertText).toBe("trim({ string: ${1:string} })");
		expect(item?.sortText?.startsWith("8_")).toBe(true);
		const edits = item?.additionalTextEdits;
		expect(edits).toHaveLength(1);
		expect(applyTestEdit(text, (edits as TestTextEdit[])[0])).toBe(
			"use string::*\n\neventsSimple onLoad() {\n\t",
		);
	});

	test("keeps the use block alphabetical and extends member lists", () => {
		const sorted = "use array::*\nuse int::*\n\neventsSimple onLoad() {\n\t";
		const item = extraCompletionItem(sorted, "trim");
		expect(
			applyTestEdit(sorted, (item?.additionalTextEdits as TestTextEdit[])[0]),
		).toBe(
			"use array::*\nuse int::*\nuse string::*\n\neventsSimple onLoad() {\n\t",
		);

		const members = "use string::{ contains }\n\neventsSimple onLoad() {\n\t";
		const trim = extraCompletionItem(members, "trim");
		expect(
			applyTestEdit(members, (trim?.additionalTextEdits as TestTextEdit[])[0]),
		).toBe("use string::{ contains, trim }\n\neventsSimple onLoad() {\n\t");
	});

	test("does not re-offer members that are already callable bare", () => {
		const text = "use string::*\n\neventsSimple onLoad() {\n\t";
		expect(extraCompletionItem(text, "trim")).toBeUndefined();
		expect(extraCompletionItem(text, "md5")).toBeDefined();
	});
});

describe("FlowScript document symbols", () => {
	test("outlines uses, interfaces, categorised variables, functions and handlers", () => {
		const text = `use string::*
use int::*

interface Report {
	title: string;
}

@category("Report")
const reportID = ""
const other = 1

function shout(s: string) {
	logInfo({ message: s })
}

eventsGeneric onLoad(payload: Struct) {
	eventsGeneric nested(payload: Struct) {
	}
}`;
		const symbols = documentSymbols(text);
		const names = symbols.map((symbol) => symbol.name);
		expect(names).toContain("use");
		expect(names).toContain("Report");
		expect(names).toContain("other");
		expect(names).toContain("shout");
		expect(names).toContain("onLoad");
		expect(names).not.toContain("reportID");

		const use = symbols.find((symbol) => symbol.name === "use");
		expect(use?.detail).toBe("2 namespaces");
		expect(use?.range.startLineNumber).toBe(1);
		expect(use?.range.endLineNumber).toBe(2);

		const iface = symbols.find(
			(symbol) => symbol.name === "Report" && symbol.kind === "interface",
		);
		expect(iface).toBeDefined();

		const category = symbols.find(
			(symbol) => symbol.name === "Report" && symbol.kind === "namespace",
		);
		expect(category?.children.map((child) => child.name)).toEqual(["reportID"]);

		const shout = symbols.find((symbol) => symbol.name === "shout");
		expect(shout?.kind).toBe("function");
		expect(shout?.detail).toBe("(s: string)");

		const onLoad = symbols.find((symbol) => symbol.name === "onLoad");
		expect(onLoad?.kind).toBe("event");
		expect(onLoad?.detail).toBe("eventsGeneric (payload: Struct)");
		expect(onLoad?.children.map((child) => child.name)).toEqual(["nested"]);
		expect(onLoad?.selectionRange.startLineNumber).toBe(16);
		expect(onLoad?.range.endLineNumber).toBe(19);
	});
});

describe("FlowScript folding", () => {
	test("folds the use block as imports and multi-line templates", () => {
		const text =
			"use string::*\nuse int::*\n\neventsSimple onLoad() {\n\tconst t = `a\nb\nc`\n}";
		const providers = registerTestProviders();
		const ranges = providers.folding()(testModel(text, editorPosition(text)));
		providers.dispose();
		expect(ranges).toContainEqual({ start: 1, end: 2, kind: "imports" });
		// Template literal opens on line 5 and closes on line 7; the closing line stays visible.
		expect(ranges.some((range) => range.start === 5 && range.end === 6)).toBe(
			true,
		);
		// The event body block still folds (the provider replaces indentation folding).
		expect(ranges.some((range) => range.start === 4 && range.end === 7)).toBe(
			true,
		);
	});
});

describe("FlowScript statement snippets", () => {
	test("scaffolds execution arms with the node's real exec output names", () => {
		const text =
			'eventsSimple onLoad() {\n\tconst r = streamCall({ prompt: "x" })\n\t';
		const item = extraCompletionItems(text).find((candidate) =>
			labelOf(candidate).startsWith("r {"),
		);
		expect(item).toBeDefined();
		expect(item?.insertText).toBe(
			"r {\n\texecSuccess: {\n\t\t$1\n\t}\n\texecError: {\n\t\t$2\n\t}\n}",
		);
		expect(labelOf(item as TestCompletionItem)).toBe(
			"r { execSuccess · execError }",
		);
	});

	test("offers for/function/event scaffolds only at statement position", () => {
		const statement = extraCompletionItems("eventsSimple onLoad() {\n\t");
		const labels = statement.map(labelOf);
		expect(labels).toContain("for … of");
		expect(labels).toContain("function …");
		expect(labels).toContain("@cache");
		const scaffold = statement.find(
			(item) => labelOf(item) === "eventsGeneric …",
		);
		expect(scaffold?.insertText).toBe(
			"eventsGeneric ${1:onEvent}() {\n\t$0\n}",
		);

		const expression = extraCompletionItems("const x = ");
		expect(expression.map(labelOf)).not.toContain("for … of");
	});

	test("stays quiet in key, enum and use-line positions", () => {
		expect(extraCompletionItems('const s = "a"\ns.contains({ ')).toEqual([]);
		expect(extraCompletionItems('acme::lookup::find("k", ')).toEqual([]);
		expect(extraCompletionItems("use str")).toEqual([]);
	});
});

describe("FlowScript inlay hints", () => {
	function hintsFor(text: string): { label: string; position: TestPosition }[] {
		const providers = registerTestProviders();
		const result = providers.inlay()(testModel(text, editorPosition(text)), {
			startLineNumber: 1,
			startColumn: 1,
			endLineNumber: text.split("\n").length,
			endColumn: 10_000,
		});
		providers.dispose();
		return result.hints.map((hint) => ({
			label: hint.label,
			position: hint.position,
		}));
	}

	test("names positional arguments and infers const binding types", () => {
		const text =
			'eventsSimple onLoad() {\n\tconst s = "  x "\n\tconst t = s.trim()\n\ts.contains("?")\n}';
		const hints = hintsFor(text);
		const labels = hints.map((hint) => hint.label);
		expect(labels).toContain("substring:");
		expect(labels).toContain(": string");
		const typeHint = hints.find((hint) => hint.label === ": string");
		expect(typeHint?.position).toEqual({ lineNumber: 3, column: 9 });
		// Literal initializers stay unannotated.
		expect(
			hints.some(
				(hint) => hint.label === ": string" && hint.position.lineNumber === 2,
			),
		).toBe(false);
	});

	test("marks impure calls buried in expression position", () => {
		const text =
			'eventsSimple onLoad() {\n\tconst s = "u"\n\tconst t = s.contains(http::fetch({ url: s }).response.body)\n}';
		const labels = hintsFor(text).map((hint) => hint.label);
		expect(labels).toContain("impure");

		const statementLevel =
			'eventsSimple onLoad() {\n\tconst r = http::fetch({ url: "u" })\n}';
		expect(hintsFor(statementLevel).map((hint) => hint.label)).not.toContain(
			"impure",
		);
	});
});

describe("FlowScript definition and references", () => {
	const text = `eventsSimple onLoad() {
	const item = "a"
	logInfo({ message: item })
	for (const item of rows) {
		logInfo({ message: item })
	}
	logInfo({ message: item })
}`;

	test("resolves shadowed bindings to the correct declaration", () => {
		const providers = registerTestProviders();
		const model = testModel(text, { lineNumber: 1, column: 1 });
		const outer = providers.definition()(
			model,
			positionOfWord(text, "item", 4),
		);
		expect(outer?.range.startLineNumber).toBe(2);
		const inner = providers.definition()(
			model,
			positionOfWord(text, "item", 3),
		);
		expect(inner?.range.startLineNumber).toBe(4);
		providers.dispose();
	});

	test("collects references without crossing shadow boundaries", () => {
		const providers = registerTestProviders();
		const model = testModel(text, { lineNumber: 1, column: 1 });
		const refs = providers.references()(
			model,
			positionOfWord(text, "item", 0),
			{
				includeDeclaration: true,
			},
		);
		expect(refs.map((ref) => ref.range.startLineNumber)).toEqual([2, 3, 7]);
		const loopRefs = providers.references()(
			model,
			positionOfWord(text, "item", 2),
			{ includeDeclaration: true },
		);
		expect(loopRefs.map((ref) => ref.range.startLineNumber)).toEqual([4, 5]);
		providers.dispose();
	});

	test("resolves use-alias names to their introducing use line", () => {
		const source = 'use acme::lookup as lk\nconst v = lk::find("k", "fast")';
		const providers = registerTestProviders();
		const model = testModel(source, { lineNumber: 1, column: 1 });
		const definition = providers.definition()(
			model,
			positionOfWord(source, "lk", 1),
		);
		expect(definition?.range.startLineNumber).toBe(1);
		expect(definition?.range.startColumn).toBe(21);
		const refs = providers.references()(
			model,
			positionOfWord(source, "lk", 1),
			{ includeDeclaration: true },
		);
		expect(refs).toHaveLength(2);
		providers.dispose();
	});

	test("definition on a catalog call returns nothing", () => {
		const providers = registerTestProviders();
		const source = 'const s = "x"\nconst t = s.trim()';
		const model = testModel(source, { lineNumber: 1, column: 1 });
		expect(
			providers.definition()(model, positionOfWord(source, "trim")),
		).toBeNull();
		providers.dispose();
	});
});

describe("FlowScript semantic tokens", () => {
	test("classifies namespaces, methods, functions, variables and locals", () => {
		const text = `use string::*
const label = ""
function myFn(p: string) {
	const local = string::trim({ string: p })
	myFn(local)
	label = local
}`;
		const tokens = decodeSemanticTokens(text);
		const at = (word: string, line: number) =>
			tokens.find((token) => token.word === word && token.line === line);

		expect(at("string", 1)?.type).toBe("namespace");
		expect(at("label", 2)).toMatchObject({
			type: "variable",
			declaration: true,
		});
		expect(at("myFn", 3)).toMatchObject({
			type: "function",
			declaration: true,
		});
		expect(at("p", 3)?.type).toBe("parameter");
		expect(at("local", 4)).toMatchObject({ type: "local", declaration: true });
		expect(at("string", 4)?.type).toBe("namespace");
		expect(at("trim", 4)?.type).toBe("method");
		expect(at("p", 4)?.type).toBe("parameter");
		expect(at("myFn", 5)).toMatchObject({
			type: "function",
			declaration: false,
		});
		expect(at("label", 6)?.type).toBe("variable");
		expect(at("local", 6)?.type).toBe("local");
	});
});

describe("FlowScript rename", () => {
	test("renames a binding across its occurrences", () => {
		const text =
			"eventsSimple onLoad() {\n\tconst count = 1\n\tlogInfo({ message: count })\n}";
		const providers = registerTestProviders();
		const model = testModel(text, { lineNumber: 1, column: 1 });
		const location = providers
			.rename()
			.resolveRenameLocation(model, positionOfWord(text, "count"));
		expect(location.text).toBe("count");
		const edit = providers
			.rename()
			.provideRenameEdits(model, positionOfWord(text, "count"), "total");
		providers.dispose();
		expect(edit.edits).toHaveLength(2);
		let result = text;
		for (const change of [...edit.edits].reverse()) {
			result = applyTestEdit(result, change.textEdit);
		}
		expect(result).toBe(
			"eventsSimple onLoad() {\n\tconst total = 1\n\tlogInfo({ message: total })\n}",
		);
	});

	test("renames a board variable model-wide but never its anchor comment", () => {
		const text =
			'const reportID = ""   //@v:abc123\n\neventsSimple onLoad() {\n\tlogInfo({ message: reportID })\n}';
		const providers = registerTestProviders();
		const model = testModel(text, { lineNumber: 1, column: 1 });
		const edit = providers
			.rename()
			.provideRenameEdits(model, positionOfWord(text, "reportID"), "reportKey");
		providers.dispose();
		expect(edit.edits).toHaveLength(2);
		let result = text;
		for (const change of [...edit.edits].reverse()) {
			result = applyTestEdit(result, change.textEdit);
		}
		expect(result).toContain('const reportKey = ""   //@v:abc123');
		expect(result).toContain("message: reportKey");
	});

	test("refuses renames that an inner shadow would capture", () => {
		const text = `eventsSimple onLoad() {
	const value = "a"
	for (const item of rows) {
		logInfo({ message: value })
	}
}`;
		const providers = registerTestProviders();
		const model = testModel(text, { lineNumber: 1, column: 1 });
		expect(() =>
			providers
				.rename()
				.provideRenameEdits(model, positionOfWord(text, "value"), "item"),
		).toThrow(/captured|collides/);
		providers.dispose();
	});

	test("refuses invalid names, keywords and catalog targets", () => {
		const text = 'const s = "x"\nconst t = s.trim()';
		const providers = registerTestProviders();
		const model = testModel(text, { lineNumber: 1, column: 1 });
		expect(() =>
			providers
				.rename()
				.provideRenameEdits(model, positionOfWord(text, "t = s"), "1abc"),
		).toThrow(/not a valid FlowScript identifier/);
		expect(() =>
			providers
				.rename()
				.provideRenameEdits(model, positionOfWord(text, "t = s"), "for"),
		).toThrow(/reserved word/);
		expect(() =>
			providers
				.rename()
				.resolveRenameLocation(model, positionOfWord(text, "trim")),
		).toThrow(/catalog/);
		providers.dispose();
	});
});
