import { describe, expect, test } from "bun:test";
import type { Monaco } from "@monaco-editor/react";
import type { INode } from "../../../lib/schema/flow/node";
import {
	computeFlowScriptDiagnostics,
	registerFlowScriptProviders,
} from "./flowscript-language";

const catalog: INode[] = [
	{
		category: "Events",
		description: "Generic workflow event",
		friendly_name: "Generic Event",
		id: "events-generic",
		name: "events_generic",
		pins: {},
	},
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
	label: string | { label: string };
}

type CompletionCallback = (
	model: TestModel,
	position: TestPosition,
) => { suggestions: TestCompletionItem[] };

function completionLabels(text: string): string[] {
	let complete: CompletionCallback | undefined;
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
			},
			CompletionItemInsertTextRule: { InsertAsSnippet: 1 },
			registerCompletionItemProvider: (
				_languageId: string,
				provider: { provideCompletionItems: CompletionCallback },
			) => {
				complete = provider.provideCompletionItems;
				return disposable;
			},
			registerHoverProvider: () => disposable,
			registerSignatureHelpProvider: () => disposable,
		},
	} as unknown as Monaco;

	const providers = registerFlowScriptProviders(monaco, () => catalog);
	if (!complete) throw new Error("Completion provider was not registered");
	const result = complete(
		{
			getValue: () => text,
			getOffsetAt: () => text.length,
			getWordUntilPosition: () => ({
				word: "",
				startColumn: 1,
				endColumn: 1,
			}),
		},
		{ lineNumber: 2, column: 1 },
	);
	providers.dispose();
	return result.suggestions.map((suggestion) =>
		typeof suggestion.label === "string"
			? suggestion.label
			: suggestion.label.label,
	);
}

describe("canonical FlowScript event headers", () => {
	test("treats the second identifier as the declared event alias", () => {
		const text = `eventsGeneric wikiExplorerLoad(payload: Struct) {
	logUnknown()
}`;
		const { markers } = computeFlowScriptDiagnostics(
			diagnosticMonaco,
			text,
			catalog,
		);
		const messages = (markers as { message: string }[]).map(
			(marker) => marker.message,
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
