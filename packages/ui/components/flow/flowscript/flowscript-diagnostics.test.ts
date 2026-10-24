import { describe, expect, test } from "bun:test";
import type {
	IFlowScriptSourceSpan,
	IFlowScriptStructuredDiagnostic,
} from "../../../state/backend-state/board-state";
import {
	flowScriptDiagnosticEntries,
	sourceSpanToRange,
} from "./flowscript-diagnostics";

function span(
	line: number,
	column: number,
	endColumn = column + 1,
	endLine = line,
): IFlowScriptSourceSpan {
	return {
		start: { line, column },
		end: { line: endLine, column: endColumn },
	};
}

function diagnostic(
	message: string,
	overrides: Partial<IFlowScriptStructuredDiagnostic> = {},
): IFlowScriptStructuredDiagnostic {
	return {
		id: message,
		code: "FS_UNKNOWN_INPUT_PIN",
		phase: "catalog_resolution",
		message,
		occurrences: 1,
		...overrides,
	};
}

describe("FlowScript diagnostic entries", () => {
	test("keeps every legacy error when the host has no structured diagnostics", () => {
		const entries = flowScriptDiagnosticEntries(["unknown", "same", "same"]);
		expect(entries.map((entry) => entry.message)).toEqual([
			"unknown",
			"same",
			"same",
		]);
		for (const entry of entries) {
			expect(entry.spans).toEqual([]);
			expect(entry.exact).toBe(false);
		}
	});

	test("shows structured groups once and retains unmatched legacy errors", () => {
		const first = span(2, 5, 10);
		const second = span(8, 5, 10);
		const entries = flowScriptDiagnosticEntries(
			["transport detail", "unknown pin", "another wording", "unknown pin"],
			[
				diagnostic("unknown pin", {
					related_messages: ["another wording"],
					occurrences: 3,
					scope: "function:send_emails",
					spans: [first, second],
					source_span: first,
				}),
			],
		);
		expect(entries).toHaveLength(2);
		expect(entries[0]).toMatchObject({
			message: "unknown pin",
			scope: "function:send_emails",
			spans: [first, second],
			exact: false,
		});
		expect(entries[1].message).toBe("transport detail");
	});

	test("retains separate exact sites for errors with identical messages", () => {
		const first = span(3, 7, 12);
		const second = span(1200, 9, 14);
		const entries = flowScriptDiagnosticEntries(
			["unknown pin", "unknown pin"],
			[
				diagnostic("unknown pin", {
					id: "first",
					scope: "function:first",
					source_span: first,
					source_span_is_exact: true,
				}),
				diagnostic("unknown pin", {
					id: "second",
					scope: "function:second",
					source_span: second,
					source_span_is_exact: true,
				}),
			],
		);
		expect(entries).toHaveLength(2);
		expect(entries.map((entry) => entry.spans)).toEqual([[first], [second]]);
		expect(entries.map((entry) => entry.scope)).toEqual([
			"function:first",
			"function:second",
		]);
		expect(entries.every((entry) => entry.exact)).toBe(true);
	});

	test("uses source_span when the sidecar has no carried span list", () => {
		const location = span(4, 8);
		const entries = flowScriptDiagnosticEntries(
			["parse error"],
			[
				diagnostic("parse error", {
					source_span: location,
					spans: [],
					source_span_is_exact: true,
				}),
			],
		);
		expect(entries[0].spans).toEqual([location]);
		expect(entries[0].exact).toBe(true);
	});

	test("keeps candidate locations distinguishable from exact locations", () => {
		const candidates = [span(3, 4), span(15, 4)];
		const entries = flowScriptDiagnosticEntries(
			["unresolved argument"],
			[
				diagnostic("unresolved argument", {
					source_span: candidates[0],
					spans: candidates,
					source_span_is_exact: false,
				}),
			],
		);
		expect(entries[0].spans).toEqual(candidates);
		expect(entries[0].exact).toBe(false);
	});

	test("ignores sidecar groups absent from the current diagnostic messages", () => {
		const entries = flowScriptDiagnosticEntries(
			["current error"],
			[
				diagnostic("previous error", {
					source_span: span(3, 4),
					source_span_is_exact: true,
				}),
			],
		);
		expect(entries).toHaveLength(1);
		expect(entries[0].message).toBe("current error");
		expect(entries[0].spans).toEqual([]);
	});
});

describe("FlowScript source ranges", () => {
	test("converts Rust scalar columns into Monaco UTF16 columns", () => {
		expect(sourceSpanToRange("🙂ab😀cd", span(1, 2, 5))).toEqual({
			startLineNumber: 1,
			startColumn: 3,
			endLineNumber: 1,
			endColumn: 7,
		});
	});

	test("converts both ends of a multiline span independently", () => {
		expect(sourceSpanToRange("x🙂z\n😀value", span(1, 3, 4, 2))).toEqual({
			startLineNumber: 1,
			startColumn: 4,
			endLineNumber: 2,
			endColumn: 5,
		});
	});

	test("accepts a location after a CRLF line ending", () => {
		expect(sourceSpanToRange("a\r\n🙂b", span(2, 2, 3))).toEqual({
			startLineNumber: 2,
			startColumn: 3,
			endLineNumber: 2,
			endColumn: 4,
		});
	});

	test("gives a zero-width parser location an inline marker", () => {
		expect(sourceSpanToRange("abc", span(1, 2, 2))).toEqual({
			startLineNumber: 1,
			startColumn: 2,
			endLineNumber: 1,
			endColumn: 3,
		});
	});

	test("expands zero-width markers across a complete Unicode codepoint", () => {
		expect(sourceSpanToRange("🙂abc", span(1, 1, 1))).toEqual({
			startLineNumber: 1,
			startColumn: 1,
			endLineNumber: 1,
			endColumn: 3,
		});
	});

	test("accepts parser positions at the end of a line and at empty EOF", () => {
		expect(sourceSpanToRange("abc", span(1, 4, 4))).toBeDefined();
		expect(sourceSpanToRange("", span(1, 1, 1))).toBeDefined();
		expect(sourceSpanToRange("abc\n", span(2, 1, 1))).toBeDefined();
	});

	test("rejects invalid or stale ranges instead of jumping to a different token", () => {
		const invalid = [
			span(0, 1),
			span(3, 1),
			span(1, 0),
			span(1, 5, 5),
			span(1, 1, 5),
			span(2, 2, 1, 1),
			span(1, 3, 2),
			span(1.5, 1),
			span(1, Number.NaN),
			span(1, 1, Number.POSITIVE_INFINITY),
		];
		for (const location of invalid) {
			expect(sourceSpanToRange("abc\ndef", location)).toBeUndefined();
		}
	});
});
