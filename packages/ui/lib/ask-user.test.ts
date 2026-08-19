import { describe, expect, it } from "bun:test";
import {
	type AskUserDraft,
	MAX_ASK_USER_QUESTIONS,
	askUserAnswerPayload,
	initialAskUserDrafts,
	isAskUserFormComplete,
	parseAskUserArguments,
	toggleAskUserChoice,
} from "./ask-user";

const intake = {
	intro: "Two things before I build this.",
	questions: [
		{
			id: "trigger",
			question: "What starts it?",
			mode: "single_choice",
			choices: [
				{ label: "A button", value: "quick_action" },
				{ label: "Every morning", value: "cron" },
			],
			default_value: "cron",
		},
		{
			id: "table",
			question: "Where should entries go?",
			mode: "freeform",
			default_value: "entries",
		},
	],
};

describe("parseAskUserArguments", () => {
	it("keeps the flat single-question shape answering under `answer`", () => {
		const form = parseAskUserArguments({
			question: "Which app?",
			mode: "single_choice",
			choices: [{ label: "Sales" }, { label: "Support" }],
			default_value: "Support",
		});

		expect(form.batched).toBe(false);
		expect(form.questions).toHaveLength(1);
		expect(form.questions[0].id).toBe("answer");

		const drafts = initialAskUserDrafts(form);
		expect(drafts[0].selected).toEqual([1]);
		expect(askUserAnswerPayload(form, drafts)).toEqual({ answer: "Support" });
	});

	it("parses a batched intake form and answers keyed by question id", () => {
		const form = parseAskUserArguments(intake);

		expect(form.batched).toBe(true);
		expect(form.intro).toBe("Two things before I build this.");
		expect(form.questions.map((question) => question.id)).toEqual([
			"trigger",
			"table",
		]);

		// Every recommended default is preselected, so sending the card unchanged is complete.
		const drafts = initialAskUserDrafts(form);
		expect(isAskUserFormComplete(form, drafts)).toBe(true);
		expect(askUserAnswerPayload(form, drafts)).toEqual({
			answers: { trigger: "cron", table: "entries" },
		});
	});

	it("treats a question with choices but no mode as single choice", () => {
		const form = parseAskUserArguments({
			question: "Which one?",
			choices: [{ label: "A" }, { label: "B" }],
		});

		expect(form.questions[0].mode).toBe("single_choice");
		// …and a declared choice mode with no choices degrades to freeform rather than rendering
		// an unanswerable block.
		const empty = parseAskUserArguments({
			question: "Which one?",
			mode: "single_choice",
		});
		expect(empty.questions[0].mode).toBe("freeform");
	});

	it("caps the batch and never lets two questions share an answer key", () => {
		const form = parseAskUserArguments({
			questions: Array.from({ length: 6 }, () => ({
				id: "same",
				question: "?",
			})),
		});

		expect(form.questions).toHaveLength(MAX_ASK_USER_QUESTIONS);
		expect(new Set(form.questions.map((question) => question.id)).size).toBe(
			MAX_ASK_USER_QUESTIONS,
		);
	});

	it("returns an empty form when there is nothing to ask", () => {
		expect(parseAskUserArguments({}).questions).toHaveLength(0);
		expect(parseAskUserArguments({ questions: [] }).questions).toHaveLength(0);
		expect(
			parseAskUserArguments({ questions: [{ id: "a" }] }).questions,
		).toHaveLength(0);
	});
});

describe("answering", () => {
	it("blocks sending until every question of a batch is answered", () => {
		const form = parseAskUserArguments({
			questions: [
				{ id: "a", question: "A?" },
				{ id: "b", question: "B?" },
			],
		});
		const drafts = initialAskUserDrafts(form);

		expect(isAskUserFormComplete(form, drafts)).toBe(false);
		drafts[0] = { ...drafts[0], text: "yes" };
		expect(isAskUserFormComplete(form, drafts)).toBe(false);
		drafts[1] = { ...drafts[1], text: "no" };
		expect(isAskUserFormComplete(form, drafts)).toBe(true);
	});

	it("replaces on single choice and accumulates on multiple choice", () => {
		const form = parseAskUserArguments({
			questions: [
				{
					id: "pick",
					question: "Pick?",
					mode: "multiple_choice",
					choices: [{ label: "A" }, { label: "B" }, { label: "C" }],
				},
			],
		});
		const question = form.questions[0];
		let draft: AskUserDraft = { text: "", selected: [] };

		draft = toggleAskUserChoice(question, draft, 2);
		draft = toggleAskUserChoice(question, draft, 0);
		expect(draft.selected).toEqual([0, 2]);
		draft = toggleAskUserChoice(question, draft, 2);
		expect(draft.selected).toEqual([0]);
		expect(askUserAnswerPayload(form, [draft])).toEqual({
			answers: { pick: ["A"] },
		});

		const single = parseAskUserArguments({
			question: "Pick?",
			mode: "single_choice",
			choices: [{ label: "A" }, { label: "B" }],
		});
		const replaced = toggleAskUserChoice(
			single.questions[0],
			{ text: "", selected: [0] },
			1,
		);
		expect(replaced.selected).toEqual([1]);
	});

	it("sends a choice's value rather than its label when both exist", () => {
		const form = parseAskUserArguments({
			question: "Store where?",
			choices: [{ label: "A new table", value: "entries" }],
		});

		expect(askUserAnswerPayload(form, [{ text: "", selected: [0] }])).toEqual({
			answer: "entries",
		});
	});
});
