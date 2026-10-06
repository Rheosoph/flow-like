import { describe, expect, test } from "bun:test";
import type { RunOutcome, RunOutput, RunStep } from "../contracts";
import { NO_TERMINAL_SIGNALS } from "./events";
import {
	EMPTY_RUN_SUMMARY,
	FIRST_LINE_MAX,
	firstTextLine,
	summaryOf,
} from "./summary";

const step = (
	number: number,
	title: string,
	state: RunStep["state"],
): RunStep => ({
	id: `step-${number}`,
	number,
	title,
	detail: null,
	message: null,
	state,
});

const output = (parts: Partial<RunOutput> = {}): RunOutput => ({
	eventCount: 5,
	steps: [],
	answer: "",
	reasoning: null,
	attachments: [],
	result: null,
	interactions: [],
	terminal: NO_TERMINAL_SIGNALS,
	...parts,
});

const failed: RunOutcome = {
	kind: "failed",
	failure: "flow",
	message: null,
	detail: null,
};

/** SP/map/real-samples.md, sample 1: the markdown answer of the medium form. */
const ANSWER = [
	"## Invoice RE-2026-0917 extracted",
	"",
	"**Vendor:** Nordwind Logistik GmbH · **Invoice date:** 17 Sep 2026",
	"",
	"| # | Description | Qty |",
	"|---|-------------|----:|",
].join("\n");

describe("firstTextLine", () => {
	test("the first line with something to read, as plain text", () => {
		expect(firstTextLine(ANSWER)).toBe("Invoice RE-2026-0917 extracted");
		expect(
			firstTextLine(
				"\n\n- **Line 4** is not on the [order](https://x.example)",
			),
		).toBe("Line 4 is not on the order");
		expect(firstTextLine("> quoted `code` and *note*")).toBe(
			"quoted code and note",
		);
		expect(firstTextLine("1. First check\n2. Second")).toBe("First check");
	});

	test("tables, rules, code blocks and empty text have no first line", () => {
		expect(firstTextLine("| a | b |\n|---|---|")).toBeNull();
		expect(firstTextLine("---\n\n```json\n{}\n```\nAfter the code")).toBe(
			"After the code",
		);
		expect(firstTextLine("   ")).toBeNull();
	});

	test("a long line is cut at a word", () => {
		const line = firstTextLine("word ".repeat(80)) ?? "";
		expect(line.length).toBeLessThanOrEqual(FIRST_LINE_MAX);
		expect(line.endsWith("word…")).toBe(true);
	});
});

describe("summaryOf", () => {
	test("a done run with an answer, steps, files and a result", () => {
		const done = output({
			answer: ANSWER,
			steps: [
				step(1, "Read documents", "done"),
				step(2, "Write report", "active"),
			],
			attachments: [
				"https://x.example/a.pdf",
				{ url: "https://x.example/b.csv" },
			],
			result: { value: { invoice_number: "RE-2026-0917" } },
		});
		expect(summaryOf(done, { kind: "succeeded" })).toEqual({
			firstLine: "Invoice RE-2026-0917 extracted",
			stepCount: 2,
			stepReached: { number: 2, title: "Write report" },
			fileCount: 2,
			hasAnswer: true,
			hasResult: true,
		});
	});

	test("a run that returned only a result shows its first row (small-done)", () => {
		const small = output({
			result: {
				value: {
					return_number: "RET-20931",
					status: "accepted",
					label: "sent by e-mail",
				},
			},
		});
		expect(summaryOf(small, { kind: "succeeded" }).firstLine).toBe(
			"Return number RET-20931",
		);
		expect(
			summaryOf(
				output({ result: { value: "Ticket 4471 created.\nMore" } }),
				null,
			).firstLine,
		).toBe("Ticket 4471 created.");
		expect(
			summaryOf(output({ result: { value: 12_483.1 } }), null).firstLine,
		).toBe("12,483.1");
	});

	test("a falsy result is a result without a first line", () => {
		const summary = summaryOf(output({ result: { value: false } }), {
			kind: "succeeded",
		});
		expect(summary.hasResult).toBe(true);
		expect(summary.firstLine).toBeNull();
	});

	test("a failed run reaches its failed step; planned steps are not counted", () => {
		const atOcr = output({
			steps: [
				step(1, "Read documents", "done"),
				step(2, "Run OCR", "active"),
				step(3, "Extract fields", "planned"),
			],
		});
		expect(summaryOf(atOcr, failed)).toMatchObject({
			stepCount: 2,
			stepReached: { number: 2, title: "Run OCR" },
			firstLine: null,
			hasAnswer: false,
		});
	});

	test("a run that never started, or has no output, keeps the empty summary", () => {
		expect(summaryOf(null, null)).toBe(EMPTY_RUN_SUMMARY);
		expect(
			summaryOf(output({ answer: "stray" }), {
				kind: "notStarted",
				reason: "declined",
			}),
		).toBe(EMPTY_RUN_SUMMARY);
		expect(summaryOf(output(), { kind: "noPlace" })).toBe(EMPTY_RUN_SUMMARY);
	});

	test("result rows use the viewer's locale and words", () => {
		const summary = summaryOf(
			output({ result: { value: { total: 10490 } } }),
			{ kind: "succeeded" },
			{ locale: "de-DE" },
		);
		expect(summary.firstLine).toBe("Total 10.490,00");
	});
});
