/*
 * A run's plan as the stage shows it: numbered steps (no known total), the active one with its
 * message, and at the end the active step marked by how the run ended. Pure.
 */
import type { IPlanStep } from "../../chat-default/chat-db";
import type { RunOutcome, RunStep, RunStepState, StepRef } from "../contracts";

const PLAN_STATE: Readonly<Record<IPlanStep["status"], RunStepState>> = {
	planned: "planned",
	progress: "active",
	done: "done",
	failed: "failed",
};

/** What the active step becomes once the run has ended; noPlace keeps its steps (it runs again). */
const ENDED_STATE: Readonly<Partial<Record<RunOutcome["kind"], RunStepState>>> =
	{
		succeeded: "done",
		failed: "failed",
		stopped: "stopped",
		notStarted: "stopped",
		unknown: "stopped",
	};

/** A URL inside a step text: its scheme colon is no title separator, and trailing punctuation is not part of it. */
const URL_SPAN = /[a-z][a-z\d+.-]*:\/\/\S*[^\s:,.;)]/gi;

interface StepText {
	readonly title: string;
	readonly detail: string | null;
}

/** "Title: description", split at the first colon outside a URL. */
export const splitStepText = (text: string): StepText => {
	const spans = Array.from(text.matchAll(URL_SPAN), (match) => {
		const start = match.index ?? 0;
		return [start, start + match[0].length] as const;
	});
	const insideUrl = (at: number) =>
		spans.some(([start, end]) => at >= start && at < end);
	for (let at = text.indexOf(":"); at > 0; at = text.indexOf(":", at + 1)) {
		if (insideUrl(at)) continue;
		return {
			title: text.slice(0, at).trim(),
			detail: text.slice(at + 1).trim() || null,
		};
	}
	return { title: text.trim(), detail: null };
};

/** The chat parser splits at the first colon, so "Fetch https://…" arrives as "Fetch https" + "//…": rejoin it. */
const stepTextOf = (step: IPlanStep): StepText => {
	const description = step.description?.trim() ?? "";
	if (description.startsWith("//"))
		return splitStepText(`${step.title}:${description}`);
	return { title: step.title.trim(), detail: description || null };
};

const stateOf = (
	step: IPlanStep,
	currentStepId: string | null | undefined,
): RunStepState => {
	if (step.id === currentStepId && step.status === "planned") return "active";
	return PLAN_STATE[step.status] ?? "planned";
};

/** The chat processor's plan steps as numbered run steps (1-based position, "Step 4"). */
export const toRunSteps = (
	planSteps: readonly IPlanStep[],
	currentStepId?: string | null,
): RunStep[] =>
	planSteps.map((step, index) => {
		const { title, detail } = stepTextOf(step);
		const message = step.reasoning?.trim() ? step.reasoning : null;
		return {
			id: step.id,
			number: index + 1,
			title,
			detail,
			message,
			state: stateOf(step, currentStepId),
		};
	});

/** At the end of a run the active step becomes done, failed or stopped; nothing else changes. */
export const markEndedSteps = (
	steps: readonly RunStep[],
	outcome: RunOutcome,
): readonly RunStep[] => {
	const ended = ENDED_STATE[outcome.kind];
	if (!ended || !steps.some((step) => step.state === "active")) return steps;
	return steps.map((step) =>
		step.state === "active" ? { ...step, state: ended } : step,
	);
};

const refOf = (step: RunStep): StepRef => ({
	number: step.number,
	title: step.title,
});

/** The step a failed run ended at ("failed at step 2: Run OCR"), or null. */
export const failedAtOf = (steps: readonly RunStep[]): StepRef | null => {
	const failed = steps.find((step) => step.state === "failed");
	return failed ? refOf(failed) : null;
};

/** The furthest step the run reached (active, failed, stopped or the last done one), or null. */
export const stepReachedOf = (steps: readonly RunStep[]): StepRef | null => {
	for (let index = steps.length - 1; index >= 0; index -= 1) {
		const step = steps[index];
		if (step && step.state !== "planned") return refOf(step);
	}
	return null;
};
