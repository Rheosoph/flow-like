/*
 * The notes above a run's steps (spec M5 anatomy, canvas `paneVM`): why a queued run waits, what
 * stopped, what failed, "Nothing has come back yet", "This run returned nothing" and what an older
 * run's receipt cannot show. Descriptors only; `copy.ts` words them. Pure.
 */
import {
	FORM_LIMITS,
	type FailureKind,
	type FormSessionState,
	type RunEntry,
	type RunStatus,
	type StepRef,
} from "../contracts";
import { elapsedMs, hasOutput } from "../run/run-view";
import { failedStepOf, stepsOf, stoppedStepOf } from "./run-steps";

/** Why a queued run waits: the API refused its start, the queue is held, or the cap of this device or plan. */
export type QueuedWhy = "noPlace" | "hold" | "device" | "plan" | "soon";

export type NoteView =
	| {
			readonly kind: "failure";
			readonly failure: FailureKind;
			readonly step: StepRef | null;
			/** A message safe to show (server text); the flow's own error text stays under Details. */
			readonly message: string | null;
			readonly detail: string | null;
			readonly runId: string | null;
			/** The form has inputs to change before trying again. */
			readonly hasFields: boolean;
			/** No file came back before it failed: "No files were saved." */
			readonly nothingSaved: boolean;
	  }
	| {
			readonly kind: "stopped";
			readonly step: number | null;
			readonly hasFields: boolean;
			/** Something came back before the stop and stays below. */
			readonly kept: boolean;
			/** A hosted page only stopped listening; the run may still finish. */
			readonly detached: boolean;
	  }
	| { readonly kind: "pending"; readonly slow: boolean }
	| { readonly kind: "nothing" }
	| { readonly kind: "queued"; readonly why: QueuedWhy; readonly cap: number }
	| {
			readonly kind: "unknown";
			/** A run of this session whose stream ended without a result; else the form was closed while it went. */
			readonly lost: boolean;
	  }
	| { readonly kind: "receipt"; readonly firstLine: string | null };

type NoteState = Pick<FormSessionState, "form" | "queue">;

/** Anything for the body to show besides notes: steps, answer text, a result or files. */
export function hasContent(run: Pick<RunEntry, "output" | "outcome">) {
	return stepsOf(run).length > 0 || hasOutput(run.output);
}

const SLOW = FORM_LIMITS.slowRunNoteMs;

export function queuedWhyOf(
	run: Pick<RunEntry, "waitingForPlace">,
	queue: FormSessionState["queue"],
): QueuedWhy {
	if (run.waitingForPlace) return "noPlace";
	if (queue.hold) return "hold";
	if (queue.cap < 0) return "soon";
	return queue.target === "local" ? "device" : "plan";
}

/** Files the run sent back: its output in this session, the saved summary for an earlier visit. */
const fileCountOf = (run: Pick<RunEntry, "output" | "summary">) =>
	run.output ? run.output.attachments.length : run.summary.fileCount;

const failureNote = (run: RunEntry, state: NoteState): NoteView | null => {
	if (run.outcome?.kind !== "failed") return null;
	return {
		kind: "failure",
		failure: run.outcome.failure,
		step: failedStepOf(run),
		message: run.outcome.message,
		detail: run.outcome.detail,
		runId: run.backendRunId,
		hasFields: state.form.fields.length > 0,
		nothingSaved: fileCountOf(run) === 0,
	};
};

const stoppedNote = (run: RunEntry, state: NoteState): NoteView => ({
	kind: "stopped",
	step: stoppedStepOf(run)?.number ?? null,
	hasFields: state.form.fields.length > 0,
	kept: hasOutput(run.output),
	detached: state.form.host.stop === "detach",
});

const LIVE_WITHOUT_QUESTION: ReadonlySet<RunStatus> = new Set<RunStatus>([
	"starting",
	"running",
	"streaming",
]);

/** A live run with nothing to show yet (the question of an asking run is its own card). */
const pendingNote = (run: RunEntry, now: number): NoteView | null => {
	if (!LIVE_WITHOUT_QUESTION.has(run.status) || hasContent(run)) return null;
	return { kind: "pending", slow: elapsedMs(run, now) >= SLOW };
};

const finishedNote = (run: RunEntry): NoteView | null => {
	if (run.status === "empty") return { kind: "nothing" };
	if (run.status !== "done" || run.output) return null;
	const { summary } = run;
	const kept =
		summary.hasAnswer ||
		summary.hasResult ||
		summary.fileCount > 0 ||
		summary.stepCount > 0;
	return kept
		? { kind: "receipt", firstLine: summary.firstLine }
		: { kind: "nothing" };
};

/** The notes of a run in the order the body shows them. */
export function notesOf(run: RunEntry, state: NoteState, now: number) {
	const notes: (NoteView | null)[] = [
		failureNote(run, state),
		run.status === "stopped" ? stoppedNote(run, state) : null,
		pendingNote(run, now),
		finishedNote(run),
		run.status === "queued"
			? {
					kind: "queued",
					why: queuedWhyOf(run, state.queue),
					cap: state.queue.cap,
				}
			: null,
		run.status === "unknown"
			? { kind: "unknown", lost: run.origin === "session" }
			: null,
	];
	return notes.filter((note): note is NoteView => note !== null);
}
