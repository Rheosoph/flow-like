/*
 * The steps a run reached, read from its output (this session) or its summary (history). The
 * active step of an ended run is marked by how the run ended (`markEndedSteps`), whether or not the
 * session already did. Pure.
 */
import type { RunEntry, RunStep, StepRef } from "../contracts";
import { failedAtOf, markEndedSteps, stepReachedOf } from "../run/steps";

const NO_STEPS: readonly RunStep[] = [];

/** The run's steps with an ended run's active step settled; none for a history run (steps are not stored). */
export function stepsOf(run: Pick<RunEntry, "output" | "outcome">) {
	if (!run.output) return NO_STEPS;
	const { steps } = run.output;
	return run.outcome ? markEndedSteps(steps, run.outcome) : steps;
}

export const stepRefOf = (step: RunStep): StepRef => ({
	number: step.number,
	title: step.title,
});

/** "Step 4: Match purchase order" while the flow works: the active step, else the furthest one reached. */
export function currentStepOf(
	run: Pick<RunEntry, "output" | "outcome" | "summary">,
) {
	const steps = stepsOf(run);
	const active = steps.find((step) => step.state === "active");
	return active
		? stepRefOf(active)
		: (stepReachedOf(steps) ?? run.summary.stepReached);
}

/** The step a failed run ended at: the session's own record, else the failed step, else nothing. */
export function failedStepOf(
	run: Pick<RunEntry, "output" | "outcome" | "failedAt">,
) {
	return run.failedAt ?? failedAtOf(stepsOf(run));
}

/** The step a stopped run was in. */
export function stoppedStepOf(
	run: Pick<RunEntry, "output" | "outcome" | "summary">,
) {
	const stopped = stepsOf(run).find((step) => step.state === "stopped");
	return stopped ? stepRefOf(stopped) : currentStepOf(run);
}
