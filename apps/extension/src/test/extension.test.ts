import * as assert from "assert";

// You can import and use all API from the 'vscode' module
// as well as import your extension to test it
import * as vscode from "vscode";
import { validateDecoratorArgument } from "../diagnostics";
import { analyzeFlowDocument } from "../flowDocument";
import { FlowDecoratorCompletionProvider, getDecorator } from "../providers";

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

		const document = await vscode.workspace.openTextDocument({
			language: "flowscript",
			content: "@ca",
		});
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
		const document = await vscode.workspace.openTextDocument({
			language: "flowscript",
			content: `@cache({ namespace: "pricing" })
function calculatePricing() {
}`,
		});
		const model = analyzeFlowDocument(document);

		assert.ok(!model.localNames.has("cache"));
		assert.ok(!model.calls.some((call) => call.name === "cache"));
		assert.ok(!model.symbols.some((symbol) => symbol.name === "cache"));
	});
});
