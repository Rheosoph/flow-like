/**
 * Shared parsing and answer-shaping for FlowPilot's `ask_user` tool.
 *
 * The tool has two argument shapes. A lone question uses the flat
 * `question`/`mode`/`choices`/`default_value`/`placeholder` fields and its answer is returned as
 * `answer`. BUILD intake instead passes a `questions` array — up to four gaps asked in ONE card,
 * each with a stable `id` — and the answers come back keyed by those ids under `answers`. Both the
 * global chat's inline prompt and the board FlowPilot's dialog render from this module, so the two
 * surfaces cannot drift apart on what a question means or what an answer looks like.
 */

export const MAX_ASK_USER_QUESTIONS = 4;

export type AskUserMode = "freeform" | "single_choice" | "multiple_choice";

export interface AskUserChoice {
	label: string;
	value?: unknown;
	description?: string;
}

export interface AskUserQuestion {
	/** Key the answer is returned under. Synthesized for the flat single-question shape. */
	id: string;
	question: string;
	mode: AskUserMode;
	choices: AskUserChoice[];
	defaultValue?: unknown;
	placeholder?: string;
}

export interface AskUserForm {
	/** Optional one-liner shown above the questions. */
	intro?: string;
	questions: AskUserQuestion[];
	/**
	 * True when the model used the `questions` array. Drives the result shape: batched forms answer
	 * with `{ answers: { [id]: value } }`, the legacy flat shape with a bare `answer`.
	 */
	batched: boolean;
}

/** Per-question working state while the card is open. */
export interface AskUserDraft {
	text: string;
	selected: number[];
}

export type AskUserAnswerPayload =
	| { answers: Record<string, unknown> }
	| { answer: unknown };

const asRecord = (value: unknown): Record<string, unknown> | undefined =>
	value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: undefined;

const asString = (value: unknown): string | undefined =>
	typeof value === "string" && value.trim().length > 0 ? value : undefined;

export const askUserChoiceValue = (choice: AskUserChoice): unknown =>
	choice.value ?? choice.label;

function parseChoices(raw: unknown): AskUserChoice[] {
	if (!Array.isArray(raw)) return [];
	return raw.flatMap((entry) => {
		const record = asRecord(entry);
		const label = record && asString(record.label);
		if (!label) return [];
		return [
			{
				label,
				value: record.value,
				description: asString(record.description),
			},
		];
	});
}

function parseQuestion(
	raw: Record<string, unknown>,
	fallbackId: string,
): AskUserQuestion | undefined {
	const question = asString(raw.question);
	if (!question) return undefined;
	const choices = parseChoices(raw.choices);
	const declared = asString(raw.mode);
	// A model that supplies choices but forgets `mode` still means "pick one" — falling back to
	// freeform there would silently drop every option it wrote.
	const mode: AskUserMode =
		declared === "single_choice" || declared === "multiple_choice"
			? declared
			: declared === "freeform" || choices.length === 0
				? "freeform"
				: "single_choice";
	return {
		id: asString(raw.id) ?? fallbackId,
		question,
		mode: mode !== "freeform" && choices.length === 0 ? "freeform" : mode,
		choices,
		defaultValue: raw.default_value ?? raw.defaultValue,
		placeholder: asString(raw.placeholder),
	};
}

/** Normalize raw `ask_user` arguments into the form both surfaces render. */
export function parseAskUserArguments(
	args: Record<string, unknown> | undefined,
): AskUserForm {
	const source = args ?? {};
	const intro = asString(source.intro);

	const batch = Array.isArray(source.questions) ? source.questions : undefined;
	if (batch?.length) {
		const questions = batch
			.slice(0, MAX_ASK_USER_QUESTIONS)
			.flatMap((entry, index) => {
				const record = asRecord(entry);
				if (!record) return [];
				const parsed = parseQuestion(record, `question_${index + 1}`);
				return parsed ? [parsed] : [];
			});
		// De-duplicate ids so two questions can never write to the same answer key.
		const seen = new Set<string>();
		for (const question of questions) {
			let id = question.id;
			let suffix = 2;
			while (seen.has(id)) id = `${question.id}_${suffix++}`;
			question.id = id;
			seen.add(id);
		}
		if (questions.length > 0) return { intro, questions, batched: true };
	}

	const single = parseQuestion(source, "answer");
	return {
		intro,
		questions: single ? [single] : [],
		batched: false,
	};
}

function defaultSelection(question: AskUserQuestion): number[] {
	if (question.mode === "freeform" || question.choices.length === 0) return [];
	const index = question.choices.findIndex(
		(choice) =>
			choice.value === question.defaultValue ||
			choice.label === question.defaultValue,
	);
	return [index >= 0 ? index : 0];
}

/** Preselect every recommended default so accepting the card unchanged is a complete answer. */
export function initialAskUserDrafts(form: AskUserForm): AskUserDraft[] {
	return form.questions.map((question) => ({
		text:
			question.mode === "freeform" && typeof question.defaultValue === "string"
				? question.defaultValue
				: "",
		selected: defaultSelection(question),
	}));
}

/** Apply a click on `choiceIndex`, honouring single- vs multiple-choice semantics. */
export function toggleAskUserChoice(
	question: AskUserQuestion,
	draft: AskUserDraft,
	choiceIndex: number,
): AskUserDraft {
	if (question.mode === "single_choice")
		return { ...draft, selected: [choiceIndex] };
	const selected = draft.selected.includes(choiceIndex)
		? draft.selected.filter((index) => index !== choiceIndex)
		: [...draft.selected, choiceIndex].sort((a, b) => a - b);
	return { ...draft, selected };
}

function isAnswered(question: AskUserQuestion, draft?: AskUserDraft): boolean {
	if (!draft) return false;
	return question.mode === "freeform"
		? draft.text.trim().length > 0
		: draft.selected.length > 0;
}

/** Every question must carry an answer before the card can be sent. */
export function isAskUserFormComplete(
	form: AskUserForm,
	drafts: AskUserDraft[],
): boolean {
	return (
		form.questions.length > 0 &&
		form.questions.every((question, index) =>
			isAnswered(question, drafts[index]),
		)
	);
}

function resolvedValue(
	question: AskUserQuestion,
	draft: AskUserDraft,
): unknown {
	if (question.mode === "freeform") return draft.text;
	const chosen = draft.selected
		.slice()
		.sort((a, b) => a - b)
		.map((index) => question.choices[index])
		.filter(Boolean)
		.map(askUserChoiceValue);
	return question.mode === "single_choice" ? (chosen[0] ?? null) : chosen;
}

/** Build the tool result payload: keyed `answers` for a batched form, a bare `answer` otherwise. */
export function askUserAnswerPayload(
	form: AskUserForm,
	drafts: AskUserDraft[],
): AskUserAnswerPayload {
	if (!form.batched) {
		const question = form.questions[0];
		const draft = drafts[0];
		return {
			answer: question && draft ? resolvedValue(question, draft) : null,
		};
	}
	const answers: Record<string, unknown> = {};
	form.questions.forEach((question, index) => {
		const draft = drafts[index];
		if (draft) answers[question.id] = resolvedValue(question, draft);
	});
	return { answers };
}
