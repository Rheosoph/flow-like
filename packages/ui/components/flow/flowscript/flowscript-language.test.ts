import { beforeAll, describe, expect, test } from "bun:test";
import type { Monaco } from "@monaco-editor/react";
import { loadFlowScriptNamesTable } from "../../../lib/flowscript/names";
import type { INode, IPin } from "../../../lib/schema/flow/node";
import {
	IPinType,
	IValueType,
	IVariableType,
} from "../../../lib/schema/flow/pin";
import {
	FLOWSCRIPT_MONARCH,
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
];

const diagnosticMonaco = {
	MarkerSeverity: { Error: 8, Warning: 4 },
} as unknown as Monaco;

interface TestPosition {
	lineNumber: number;
	column: number;
}

interface TestModel {
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
	documentation?: { value: string };
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
	let complete: CompletionCallback | undefined;
	let hover: HoverCallback | undefined;
	let signature: SignatureCallback | undefined;
	const disposable = { dispose: () => undefined };
	const monaco = {
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
			},
			CompletionItemInsertTextRule: { InsertAsSnippet: 1 },
			registerCompletionItemProvider: (
				_languageId: string,
				provider: { provideCompletionItems: CompletionCallback },
			) => {
				complete = provider.provideCompletionItems;
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
		},
	} as unknown as Monaco;

	const providers = registerFlowScriptProviders(monaco, () => catalog);
	return {
		complete: () => {
			if (!complete) throw new Error("Completion provider was not registered");
			return complete;
		},
		hover: () => {
			if (!hover) throw new Error("Hover provider was not registered");
			return hover;
		},
		signature: () => {
			if (!signature) throw new Error("Signature provider was not registered");
			return signature;
		},
		dispose: providers.dispose,
	};
}

function testModel(text: string, position: TestPosition): TestModel {
	return {
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

function diagnosticMessages(text: string): string[] {
	const { markers } = computeFlowScriptDiagnostics(
		diagnosticMonaco,
		text,
		catalog,
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
