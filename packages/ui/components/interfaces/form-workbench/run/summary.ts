/*
 * What lists and history keep of a run without its output (spec §5: no answers or results are
 * stored): the first line of the answer, else of the result, how far the steps got and how many
 * files came back. Pure.
 */
import { plainMarkdownLines } from "../../../../lib/markdown-text";
import type { RunOutcome, RunOutput, RunSummary } from "../contracts";
import { type ResultFormat, leadRowOf, toResultModel } from "./result-view";
import { markEndedSteps, stepReachedOf } from "./steps";

export const EMPTY_RUN_SUMMARY: RunSummary = {
	firstLine: null,
	stepCount: 0,
	stepReached: null,
	fileCount: 0,
	hasAnswer: false,
	hasResult: false,
};

/** A first line longer than this is cut at a word with "…" (the Runs list shows one line). */
export const FIRST_LINE_MAX = 160;

function clip(text: string) {
	if (text.length <= FIRST_LINE_MAX) return text;
	const cut = text.slice(0, FIRST_LINE_MAX - 1);
	const space = cut.lastIndexOf(" ");
	return `${space > FIRST_LINE_MAX / 2 ? cut.slice(0, space) : cut}…`;
}

/** The first line of markdown with something to read, as plain text ("## Invoice extracted" → "Invoice extracted"). */
export function firstTextLine(text: string) {
	const line = plainMarkdownLines(text).next().value;
	return line ? clip(line) : null;
}

/** A text result's first line, a number as shown, an object's first text or number row ("Return number RET-20931"). */
function resultLine(value: unknown, format?: ResultFormat) {
	if (typeof value === "string") return firstTextLine(value);
	if (typeof value === "number") {
		const { view } = toResultModel(value, format);
		return view.kind === "value" ? view.text : null;
	}
	const row = leadRowOf(value, format);
	return row ? clip(`${row.label} ${row.text}`) : null;
}

function firstLineOf(output: RunOutput, format?: ResultFormat) {
	const answer = firstTextLine(output.answer);
	if (answer) return answer;
	return output.result ? resultLine(output.result.value, format) : null;
}

/**
 * A run's summary for the Runs list and its stored record. A run that never started keeps the
 * empty summary; `format` carries the viewer's locale and words for result rows.
 */
export const summaryOf = (
	output: RunOutput | null,
	outcome: RunOutcome | null,
	format?: ResultFormat,
): RunSummary => {
	if (!output || outcome?.kind === "notStarted" || outcome?.kind === "noPlace")
		return EMPTY_RUN_SUMMARY;
	const steps = outcome ? markEndedSteps(output.steps, outcome) : output.steps;
	return {
		firstLine: firstLineOf(output, format),
		stepCount: steps.filter((step) => step.state !== "planned").length,
		stepReached: stepReachedOf(steps),
		fileCount: output.attachments.length,
		hasAnswer: output.answer.trim() !== "",
		hasResult: output.result !== null,
	};
};
