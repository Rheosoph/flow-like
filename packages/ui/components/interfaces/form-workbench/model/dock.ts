import {
	BUSY_RUN_STATUSES,
	type DockLine,
	type DockLineInput,
	type DockLineOf,
	type FieldValues,
	type FileSlot,
	type FormSessionState,
	type QueueSummary,
	type WorkbenchField,
} from "../contracts";
import { unseenFailures } from "./queue";

type DockRule = (input: DockLineInput) => DockLine | null;

const sendingLine: DockRule = (input) => {
	const sending = input.sending;
	if (!sending || sending.left <= 0) return null;
	const total = Math.max(sending.total, sending.left);
	return {
		kind: "sending",
		current: Math.min(total, total - sending.left + 1),
		total,
	};
};

const comparedRule: DockRule = (input) =>
	input.compared
		? { kind: "compared", n: input.compared.n, changes: input.compared.changes }
		: null;

/** The status line's priority, highest first (spec M5, `flpDockLine`). */
const DOCK_RULES: readonly DockRule[] = [
	sendingLine,
	(input) =>
		input.problems > 0 ? { kind: "problems", count: input.problems } : null,
	(input) => (input.hold ? { kind: "hold", hold: input.hold } : null),
	(input) => (input.message ? { kind: "message", entry: input.message } : null),
	(input) =>
		input.failures.length > 0
			? { kind: "failure", runs: input.failures }
			: null,
	(input) =>
		input.question ? { kind: "question", question: input.question } : null,
	(input) =>
		input.queued > 0 || input.running > 1
			? { kind: "queue", running: input.running, queued: input.queued }
			: null,
	(input) =>
		input.blocked.length > 0
			? { kind: "blocked", label: input.blocked[0] }
			: null,
	comparedRule,
	(input) =>
		input.missing > 0 ? { kind: "missing", count: input.missing } : null,
];

/**
 * The dock's one status line, highest first: files of the current entry sending · fields that need a look ·
 * the hold · a message · unseen failures · the offer or series-end question · the queue line (2+ running or
 * any queued) · a required file field this page cannot fill · the comparison · fields to fill in · Ready.
 */
export const dockLine: DockLineOf = (input) => {
	for (const rule of DOCK_RULES) {
		const line = rule(input);
		if (line) return line;
	}
	return { kind: "ready" };
};

const isFileSlot = (value: unknown): value is FileSlot =>
	typeof value === "object" &&
	value !== null &&
	typeof (value as FileSlot).id === "string" &&
	typeof (value as FileSlot).state === "string";

function slotsOf(value: unknown) {
	const list: readonly unknown[] = Array.isArray(value) ? value : [value];
	return list.filter(isFileSlot);
}

/** The rail's own file slots (file and files fields); next files are not part of the entry. */
function railSlots(fields: readonly WorkbenchField[], values: FieldValues) {
	return fields
		.filter((field) => field.kind === "file" || field.kind === "files")
		.flatMap((field) => slotsOf(values[field.name]));
}

/** A sent file belongs to the current entry when it was sent after the last press (or before any press). */
function sentForEntry(slot: FileSlot, lastPressAt: number | null) {
	if (slot.state !== "sent") return false;
	return (
		lastPressAt === null || slot.sentAt === null || slot.sentAt >= lastPressAt
	);
}

/**
 * Files of the current entry still to send, and how many belong to the entry: those still to send plus those
 * sent since the last press (all sent ones before the first press). Failed files and "Pick again" reminders
 * are not sending.
 */
function entrySending(state: FormSessionState) {
	let left = 0;
	let sent = 0;
	for (const slot of railSlots(state.form.fields, state.rail.values)) {
		if (slot.state === "waiting" || slot.state === "sending") left++;
		else if (sentForEntry(slot, state.rail.lastPressAt)) sent++;
	}
	return left > 0 ? { left, total: left + sent } : null;
}

/** `names` come from `blockedNames`; the dock names the required ones by their labels. */
function blockedLabels(
	fields: readonly WorkbenchField[],
	names: readonly string[],
) {
	return fields
		.filter((field) => field.required && names.includes(field.name))
		.map((field) => field.label);
}

function comparedOf(state: FormSessionState, changes: number | null) {
	const id = state.rail.comparedRunId;
	if (changes === null || id === null || state.form.fields.length === 0)
		return null;
	const run = state.runs.find((candidate) => candidate.id === id);
	return run ? { n: run.n, changes } : null;
}

/** Runs holding a place, queued, sending, and whether the queue is on hold. */
export const queueSummary = (state: FormSessionState): QueueSummary => {
	const count = (wanted: (status: string) => boolean) =>
		state.runs.filter((run) => wanted(run.status)).length;
	return {
		busy: count((status) => BUSY_RUN_STATUSES.some((busy) => busy === status)),
		queued: count((status) => status === "queued"),
		sending: count((status) => status === "sending"),
		held: state.queue.hold !== null,
	};
};

/**
 * The status line's input from the session state. `derived` comes from the M-INPUTS helpers:
 * `comparedChanges` = the number of `diffValues(…)` against `rail.comparedRunId`'s copy without per-run fields
 * (null: no comparison), `missing` = `missingCount(…)`, `blocked` = `blockedNames(fields, host)`.
 */
export const dockLineInputOf = (
	state: FormSessionState,
	derived: {
		readonly comparedChanges: number | null;
		readonly missing: number;
		readonly blocked: readonly string[];
	},
): DockLineInput => {
	const summary = queueSummary(state);
	return {
		sending: entrySending(state),
		problems: state.rail.pressed ? Object.keys(state.rail.problems).length : 0,
		hold: state.queue.hold,
		message: state.view.message,
		failures: unseenFailures(state.runs),
		question: state.view.question,
		running: summary.busy,
		queued: summary.queued,
		blocked: blockedLabels(state.form.fields, derived.blocked),
		compared: comparedOf(state, derived.comparedChanges),
		missing: derived.missing,
	};
};
