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
	insertText?: string;
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
			registerHoverProvider: (
				_languageId: string,
				provider: { provideHover: HoverCallback },
			) => {
				hover = provider.provideHover;
				return disposable;
			},
			registerSignatureHelpProvider: () => disposable,
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
		dispose: providers.dispose,
	};
}

function completionItems(text: string): TestCompletionItem[] {
	const providers = registerTestProviders();
	const position = editorPosition(text);
	const complete = providers.complete();
	if (!complete) throw new Error("Completion provider was not registered");
	const result = complete(
		{
			getValue: () => text,
			getOffsetAt: (at) => offsetAt(text, at),
			getWordUntilPosition: () => ({
				word: "",
				startColumn: position.column,
				endColumn: position.column,
			}),
		},
		position,
	);
	providers.dispose();
	return result.suggestions;
}

function completionLabels(text: string): string[] {
	return completionItems(text).map((suggestion) =>
		typeof suggestion.label === "string"
			? suggestion.label
			: suggestion.label.label,
	);
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
			getValue: () => text,
			getOffsetAt: (at) => offsetAt(text, at),
			getWordUntilPosition: () => ({
				word: "",
				startColumn: position.column,
				endColumn: position.column,
			}),
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
					range.endColumn - 1,
				);
			},
		},
		position,
	);
	providers.dispose();
	return result?.contents[0]?.value;
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

describe("FlowScript function cache editor support", () => {
	test("offers bare and configured cache decorator snippets", () => {
		const items = completionItems("@ca");
		const labels = items.map((item) =>
			typeof item.label === "string" ? item.label : item.label.label,
		);

		expect(labels).toEqual(["@cache", "@cache({ … })"]);
		expect(items[1]?.insertText).toBe(
			'@cache({ namespace: "${1:global}", ttlSeconds: ${2:300}, scope: "${3|app,user|}" })',
		);
	});

	test("offers only missing cache settings inside the decorator", () => {
		const items = completionItems('@cache({ namespace: "pricing", ');
		const labels = items.map((item) =>
			typeof item.label === "string" ? item.label : item.label.label,
		);

		expect(labels).toEqual(["ttlSeconds", "scope"]);
		expect(labels).not.toContain("eventsGeneric");
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
		const { markers } = computeFlowScriptDiagnostics(
			diagnosticMonaco,
			text,
			catalog,
		);

		expect(markers).toHaveLength(0);
	});

	test("keeps catalog completions available outside cache settings", () => {
		expect(completionLabels("function calculatePricing() {\n\t")).toContain(
			"eventsGeneric",
		);
	});
});
