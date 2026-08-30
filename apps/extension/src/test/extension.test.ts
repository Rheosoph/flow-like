import * as assert from "assert";

// You can import and use all API from the 'vscode' module
// as well as import your extension to test it
import * as vscode from "vscode";
import { unknownCallIssues, validateDecoratorArgument } from "../diagnostics";
import { analyzeFlowDocument, parseUseDeclarations } from "../flowDocument";
import {
	FlowCompletionProvider,
	FlowDecoratorCompletionProvider,
	FlowHoverProvider,
	FlowMemberCompletionProvider,
	FlowPathCompletionProvider,
	FlowSignatureHelpProvider,
	getDecorator,
} from "../providers";
import { SignatureRegistry, parseDeclarations, signatureLabel } from "../signatures";
import { DECLARATIONS, NAMES, SCHEMAS } from "./fixtures";

function registryWithDeclarations(withNames = true): SignatureRegistry {
	const registry = new SignatureRegistry();
	if (withNames) {
		registry.ingestNames(NAMES);
	}
	registry.ingest(vscode.Uri.parse("untitled:catalog.flow.d"), DECLARATIONS);
	registry.ingestSchemas(SCHEMAS);
	return registry;
}

async function flowDocument(content: string): Promise<vscode.TextDocument> {
	return vscode.workspace.openTextDocument({ language: "flowscript", content });
}

function endOf(document: vscode.TextDocument): vscode.Position {
	return document.lineAt(document.lineCount - 1).range.end;
}

function labelOf(item: vscode.CompletionItem): string {
	return typeof item.label === "string" ? item.label : item.label.label;
}

function snippetOf(item: vscode.CompletionItem | undefined): string | undefined {
	return item?.insertText instanceof vscode.SnippetString
		? item.insertText.value
		: typeof item?.insertText === "string"
			? item.insertText
			: undefined;
}

function positionOf(document: vscode.TextDocument, needle: string, skip = 0): vscode.Position {
	const text = document.getText();
	let offset = -1;
	for (let i = 0; i <= skip; i++) {
		offset = text.indexOf(needle, offset + 1);
	}
	assert.ok(offset >= 0, `'${needle}' not found`);
	return document.positionAt(offset);
}

suite("Extension Test Suite", () => {
	vscode.window.showInformationMessage("Start all tests.");

	test("Sample test", () => {
		assert.strictEqual(-1, [1, 2, 3].indexOf(5));
		assert.strictEqual(-1, [1, 2, 3].indexOf(0));
	});

	test("cache decorator exposes configured completion and documentation", async () => {
		const definition = getDecorator("cache");
		assert.ok(definition);
		assert.strictEqual(definition.argumentKind, "optional-cache-settings");
		assert.match(definition.doc, /namespace/);
		assert.match(definition.doc, /ttlSeconds/);
		assert.match(definition.doc, /app.*user/);
		assert.match(definition.doc, /global/);
		assert.match(definition.doc, /300-second/);
		assert.match(definition.doc, /0.*no expiry/);

		const document = await flowDocument("@ca");
		const items = new FlowDecoratorCompletionProvider().provideCompletionItems(
			document,
			new vscode.Position(0, 3),
		);
		const cache = items?.find((item) => item.label === "@cache");
		assert.ok(cache);
		assert.ok(cache.insertText instanceof vscode.SnippetString);
		assert.match(cache.insertText.value, /namespace:.*global/);
		assert.match(cache.insertText.value, /ttlSeconds:.*300/);
		assert.match(cache.insertText.value, /app,user/);
	});

	test("cache decorator lint accepts bare and object forms", () => {
		assert.strictEqual(
			validateDecoratorArgument("cache", undefined),
			undefined,
		);
		assert.strictEqual(validateDecoratorArgument("cache", "({})"), undefined);
		assert.strictEqual(
			validateDecoratorArgument(
				"cache",
				'({ namespace: "pricing", ttlSeconds: 0, scope: "user" })',
			),
			undefined,
		);

		const invalid = validateDecoratorArgument("cache", '("pricing")');
		assert.ok(invalid);
		assert.match(invalid.message, /settings object/);
		assert.match(invalid.message, /ttlSeconds: 0/);
		assert.doesNotMatch(invalid.message, /requires a string argument/);
	});

	test("cache decorator is not indexed as an event or function call", async () => {
		const document = await flowDocument(`@cache({ namespace: "pricing" })
function calculatePricing() {
}`);
		const model = analyzeFlowDocument(document);

		assert.ok(!model.localNames.has("cache"));
		assert.ok(!model.calls.some((call) => call.name === "cache"));
		assert.ok(!model.symbols.some((symbol) => symbol.name === "cache"));
	});
});

suite("Declaration registry", () => {
	test("parses legacy and namespaced declarations into one node-keyed registry", () => {
		const sigs = parseDeclarations(vscode.Uri.parse("untitled:x.flow.d"), DECLARATIONS);
		const contains = sigs.find((sig) => sig.nodeType === "string_contains");
		assert.ok(contains);
		assert.strictEqual(contains.name, "string::contains");
		assert.strictEqual(contains.flat, "stringContains");
		assert.deepStrictEqual(contains.namespace, ["string"]);
		assert.strictEqual(contains.alias, "contains");
		assert.strictEqual(contains.receiver, "string");
		assert.strictEqual(contains.receiverClass, "string");
		assert.deepStrictEqual(
			contains.params.map((p) => `${p.name}${p.optional ? "?" : ""}: ${p.type}`),
			["string: string", "substring: string", "ignoreCase?: bool"],
		);
		assert.strictEqual(contains.params[1].doc, "Needle");
		assert.strictEqual(
			signatureLabel(contains),
			"string::contains({ string: string, substring: string, ignoreCase?: bool }): bool",
		);
		assert.strictEqual(
			signatureLabel(contains, true),
			"string.contains({ substring: string, ignoreCase?: bool }): bool",
		);

		const read = sigs.find((sig) => sig.nodeType === "ai_ml_model_read");
		assert.ok(read);
		assert.deepStrictEqual(read.namespace, ["ai", "ml"]);
		assert.strictEqual(read.name, "ai::ml::read");

		const toText = sigs.find((sig) => sig.nodeType === "http_response_to_text");
		assert.ok(toText);
		assert.strictEqual(toText.receiverClass, "HttpResponse");
		assert.deepStrictEqual(toText.params.map((p) => p.name), ["response"]);

		const legacy = sigs.find((sig) => sig.flat === "stringTrim");
		assert.ok(legacy);
		assert.strictEqual(legacy.name, "stringTrim");
		assert.strictEqual(legacy.namespace, undefined);
	});

	test("names.json enriches legacy declarations and every spelling resolves", () => {
		const registry = registryWithDeclarations();
		const trim = registry.get("stringTrim");
		assert.ok(trim);
		assert.strictEqual(trim.nodeType, "string_trim");
		assert.strictEqual(trim.name, "string::trim");
		assert.strictEqual(registry.get("string::trim"), trim);
		assert.strictEqual(registry.get("string_trim"), trim);
		assert.strictEqual(registry.member(["string"], "trim"), trim);
		assert.deepStrictEqual(registry.methodCandidates("string", "trim"), [trim]);
		assert.deepStrictEqual(registry.methodCandidates("int", "trim"), []);
		assert.strictEqual(registry.methodCandidates(undefined, "trim").length, 1);
		assert.strictEqual(registry.get("hash::md5")?.flat, "utilsHashMd5");
		assert.strictEqual(registry.member(["ai", "ml"], "read")?.nodeType, "ai_ml_model_read");
		assert.ok(registry.namespace(["ai"])?.children.has("ml"));
		assert.strictEqual(registry.schemasFor("http::fetch")?.outputs?.response !== undefined, true);
		assert.strictEqual(registry.schemasFor(registry.get("httpFetch")!)?.outputs?.response !== undefined, true);
	});
});

suite("FlowScript document model", () => {
	test("classifies bare, path and method calls and parses use lines", async () => {
		const document = await flowDocument(`use ai::ml
use string::*, ui::{ setElementText, navigateTo }
use data::atlassian::jira as jira

eventsSimple onLoad() {
	const s = "  hi  "
	const t = stringTrim({ string: s })
	const u = string::trim({ string: s })
	const v = s.trim()
	const w = ml::read({ path: "p" })
	const { hash: h, other } = hash::md5({ input: s })
	for (const [i, item] of items) { log::info(\`\${item.trim()} \${i}\`) }
}`);
		const model = analyzeFlowDocument(document);
		const byDisplay = new Map(model.calls.map((call) => [call.display, call]));
		assert.strictEqual(byDisplay.get("stringTrim")?.kind, "bare");
		assert.strictEqual(byDisplay.get("string::trim")?.kind, "path");
		assert.deepStrictEqual(byDisplay.get("string::trim")?.path, ["string"]);
		assert.strictEqual(byDisplay.get("s.trim")?.kind, "method");
		assert.strictEqual(byDisplay.get("s.trim")?.receiverText, "s");
		assert.strictEqual(byDisplay.get("ml::read")?.kind, "path");
		assert.strictEqual(byDisplay.get("hash::md5")?.kind, "path");
		assert.strictEqual(byDisplay.get("item.trim")?.kind, "method");
		assert.strictEqual(byDisplay.get("log::info")?.kind, "path");
		assert.ok(!model.calls.some((call) => call.name === "string"));

		assert.deepStrictEqual(
			model.uses.map((use) => use.kind),
			["namespace", "glob", "members", "alias"],
		);
		assert.deepStrictEqual(model.uses[0].path, ["ai", "ml"]);
		assert.strictEqual(document.getText(model.uses[2].range), "ui::{ setElementText, navigateTo }");
		assert.deepStrictEqual(parseUseDeclarations(document).map((use) => use.path), [
			["ai", "ml"],
			["string"],
			["ui"],
			["data", "atlassian", "jira"],
		]);

		const names = model.variables.map((variable) => variable.name);
		for (const expected of ["s", "t", "u", "v", "w", "h", "other", "i", "item"]) {
			assert.ok(names.includes(expected), `variable ${expected}`);
		}
		const h = model.variables.find((variable) => variable.name === "h");
		assert.strictEqual(h?.initCall, "hash::md5");
		assert.strictEqual(h?.initField, "hash");
		assert.strictEqual(model.variables.find((v) => v.name === "v")?.initMethod?.member, "trim");
		assert.strictEqual(model.variables.find((v) => v.name === "item")?.iterates, "items");
		assert.strictEqual(model.variables.find((v) => v.name === "i")?.typeText, "int");
	});
});

suite("FlowScript diagnostics", () => {
	test("legacy flat calls and every new spelling stay clean", async () => {
		const registry = registryWithDeclarations();
		const document = await flowDocument(`use string::*
use ai::ml

function shout(s: string): (out: string) { return s }

eventsSimple onLoad() {
	const s = "  hi  "
	const t = stringTrim({ string: s })
	const u = string::trim({ string: s })
	const v = s.trim()
	const w = "lit".contains("?", { ignoreCase: true })
	const x = trim({ string: s })
	const y = hash::md5({ input: s })
	const z = ml::read({ path: "p" })
	const n = (5).abs()
	const r = http::fetch({ url: "u" })
	const body = r.responseToText()
	const msg = \`Hello \${s.trim()} and \${mystery()}\`
	const shouted = s.shout()
	const o = string.length({ string: s })
	let count = 0
	count += 1
	@parallel for (const [i, item] of items) {
		logInfo({ message: item })
	}
	while (!done) { log::info({ message: msg }); }
}`);
		const issues = unknownCallIssues(analyzeFlowDocument(document), registry);
		assert.deepStrictEqual(
			issues.map((issue) => issue.message),
			[],
		);
	});

	test("flags unknown namespaces, members and methods with spelling hints", async () => {
		const registry = registryWithDeclarations();
		const document = await flowDocument(`eventsSimple onLoad() {
	const s = "x"
	const a = nope::thing()
	const b = array::trim({ string: s })
	const c = s.abs()
	const d = trim({ string: s })
	const e = s.nothing()
}`);
		const messages = unknownCallIssues(analyzeFlowDocument(document), registry).map(
			(issue) => issue.message,
		);
		assert.strictEqual(messages.length, 5);
		assert.match(messages[0], /Unknown function 'nope::thing'/);
		assert.match(messages[0], /namespace 'nope'/);
		assert.match(messages[1], /'trim' is not a member of namespace 'array'/);
		assert.match(messages[1], /`string::trim\(…\)`/);
		assert.match(messages[2], /Unknown method 'abs' on string/);
		assert.match(messages[2], /`int::abs\(…\)`/);
		assert.match(messages[3], /Unknown function 'trim'/);
		assert.match(messages[3], /`string::trim\(…\)`/);
		assert.match(messages[4], /Unknown method 'nothing' on string/);
	});

	test("keeps working without names.json (legacy declarations only)", async () => {
		const registry = registryWithDeclarations(false);
		const document = await flowDocument(`eventsSimple onLoad() {
	const t = stringTrim({ string: "x" })
	const c = string::contains({ string: "x", substring: "?" })
	const u = unknownThing()
}`);
		const messages = unknownCallIssues(analyzeFlowDocument(document), registry).map(
			(issue) => issue.message,
		);
		assert.strictEqual(messages.length, 1);
		assert.match(messages[0], /Unknown function 'unknownThing'/);
		assert.strictEqual(registry.get("stringTrim")?.name, "stringTrim");
	});
});

suite("FlowScript providers", () => {
	test("completes namespace members after `::` with qualified-form snippets", async () => {
		const registry = registryWithDeclarations();
		const document = await flowDocument("use ai::ml\neventsSimple onLoad() {\n\tstring::");
		const items =
			new FlowPathCompletionProvider(registry).provideCompletionItems(
				document,
				endOf(document),
			) ?? [];
		const labels = items.map(labelOf);
		assert.ok(labels.includes("trim"));
		assert.ok(labels.includes("contains"));
		assert.ok(!labels.includes("md5"));
		assert.strictEqual(
			snippetOf(items.find((item) => labelOf(item) === "contains")),
			"contains({ string: ${1:string}, substring: ${2:substring}, ignoreCase: ${3:ignoreCase} })",
		);

		const nested = await flowDocument("use ai::ml\nml::");
		const nestedLabels = (
			new FlowPathCompletionProvider(registry).provideCompletionItems(nested, endOf(nested)) ??
			[]
		).map(labelOf);
		assert.deepStrictEqual(nestedLabels, ["read"]);

		const root = await flowDocument("ai::");
		const rootItems =
			new FlowPathCompletionProvider(registry).provideCompletionItems(root, endOf(root)) ?? [];
		assert.deepStrictEqual(rootItems.map(labelOf), ["ml"]);
		assert.strictEqual(rootItems[0].insertText, "ml::");
	});

	test("completes methods after `.` by receiver class, plus fields and bare-position spellings", async () => {
		const registry = registryWithDeclarations();
		const provider = new FlowMemberCompletionProvider(registry);

		const stringDoc = await flowDocument('const s = "x"\nconst t = s.');
		const stringItems = provider.provideCompletionItems(stringDoc, endOf(stringDoc)) ?? [];
		const stringLabels = stringItems.map(labelOf);
		assert.ok(stringLabels.includes("trim"));
		assert.ok(stringLabels.includes("contains"));
		assert.ok(!stringLabels.includes("abs"));
		assert.strictEqual(
			snippetOf(stringItems.find((item) => labelOf(item) === "contains")),
			"contains({ substring: ${1:substring}, ignoreCase: ${2:ignoreCase} })",
		);
		assert.strictEqual(snippetOf(stringItems.find((item) => labelOf(item) === "trim")), "trim()");

		const intDoc = await flowDocument('const s = "x"\nconst n = s.length().');
		const intLabels = (provider.provideCompletionItems(intDoc, endOf(intDoc)) ?? []).map(labelOf);
		assert.ok(intLabels.includes("abs"));
		assert.ok(!intLabels.includes("trim"));

		const responseDoc = await flowDocument('const r = http::fetch({ url: "u" })\nconst b = r.');
		const responseLabels = (
			provider.provideCompletionItems(responseDoc, endOf(responseDoc)) ?? []
		).map(labelOf);
		assert.ok(responseLabels.includes("status"));
		assert.ok(responseLabels.includes("body"));
		assert.ok(responseLabels.includes("responseToText"));
		assert.ok(!responseLabels.includes("trim"));

		const unknownDoc = await flowDocument("const t = mystery.");
		const unknownItems = provider.provideCompletionItems(unknownDoc, endOf(unknownDoc)) ?? [];
		const trim = unknownItems.find((item) => labelOf(item) === "trim");
		assert.ok(trim);
		assert.ok(unknownItems.some((item) => labelOf(item) === "abs"));
		assert.match(typeof trim.label === "string" ? "" : (trim.label.description ?? ""), /string/);

		const bare = new FlowCompletionProvider(registry);
		const closedDoc = await flowDocument("eventsSimple onLoad() {\n\t");
		const closedItems = bare.provideCompletionItems(closedDoc, endOf(closedDoc));
		const qualified = closedItems.find((item) => labelOf(item) === "string::trim");
		assert.ok(qualified);
		assert.strictEqual(snippetOf(qualified), "string::trim({ string: ${1:string} })");
		assert.match(qualified.filterText ?? "", /stringTrim/);
		assert.ok(closedItems.some((item) => labelOf(item) === "string"));
		assert.ok(!closedItems.some((item) => labelOf(item) === "trim"));

		const openedDoc = await flowDocument("use string::*\n\t");
		const openedItems = bare.provideCompletionItems(openedDoc, endOf(openedDoc));
		assert.strictEqual(
			snippetOf(openedItems.find((item) => labelOf(item) === "trim")),
			"trim({ string: ${1:string} })",
		);
		assert.ok(openedItems.some((item) => labelOf(item) === "hash::md5"));
	});

	test("hover resolves flat, qualified and method spellings to the same node", async () => {
		const registry = registryWithDeclarations();
		const hover = new FlowHoverProvider(registry);
		const document = await flowDocument(`use string::*
const s = "x"
const a = stringTrim({ string: s })
const b = string::trim({ string: s })
const c = s.trim()
const d = trim({ string: s })
const e = hash::md5({ input: s })`);
		const expectNode = (needle: string, skip = 0) => {
			const result = hover.provideHover(document, positionOf(document, needle, skip));
			assert.ok(result, `hover for ${needle}`);
			const md = (result.contents[0] as vscode.MarkdownString).value;
			assert.match(md, /string::trim\(\{ string: string \}\): string/);
			assert.match(md, /legacy `stringTrim\(…\)`/);
			assert.match(md, /x\.trim\(\)/);
		};
		expectNode("stringTrim");
		expectNode("trim(", 0);
		expectNode("trim()");
		expectNode("trim({ string: s })", 1);

		const ns = hover.provideHover(document, positionOf(document, "hash::"));
		assert.ok(ns);
		assert.match((ns.contents[0] as vscode.MarkdownString).value, /use hash::\*/);
	});

	test("signature help hides the receiver in method form", async () => {
		const registry = registryWithDeclarations();
		const provider = new FlowSignatureHelpProvider(registry);
		const methodDoc = await flowDocument('const s = "x"\nconst c = s.contains({ ');
		const method = provider.provideSignatureHelp(methodDoc, endOf(methodDoc));
		assert.strictEqual(
			method?.signatures[0].label,
			"string.contains({ substring: string, ignoreCase?: bool }): bool",
		);
		assert.strictEqual(method?.signatures[0].parameters.length, 2);

		const staticDoc = await flowDocument('const c = string::contains({ ');
		const stat = provider.provideSignatureHelp(staticDoc, endOf(staticDoc));
		assert.strictEqual(
			stat?.signatures[0].label,
			"string::contains({ string: string, substring: string, ignoreCase?: bool }): bool",
		);
	});
});
