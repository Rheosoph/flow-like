/*
 * Derivations the stage, the dock and the Runs list share: which run is on the stage, its clock,
 * whether it can be run again exactly (S4), the status its outcome gives, and what a live run is
 * doing. Pure.
 */
import {
	type CopyValue,
	type FileSlot,
	type FileSlotState,
	type FormSessionState,
	LIVE_RUN_STATUSES,
	type RunEntry,
	type RunOutcome,
	type RunOutput,
	type RunStatus,
	type WorkbenchField,
} from "../contracts";
import { buildPayload } from "../model/payload";

/** Runs that wait for a place or for their files never take the stage by themselves (M5). */
const WAITING: ReadonlySet<RunStatus> = new Set<RunStatus>([
	"queued",
	"sending",
]);
const LIVE: ReadonlySet<RunStatus> = new Set(LIVE_RUN_STATUSES);
const SLOT_STATES: ReadonlySet<string> = new Set<FileSlotState>([
	"waiting",
	"sending",
	"sent",
	"failed",
	"reminder",
]);

const STATUS_OF_OUTCOME: Readonly<
	Record<RunOutcome["kind"], (output: RunOutput | null) => RunStatus>
> = {
	succeeded: (output) => (hasOutput(output) ? "done" : "empty"),
	failed: () => "failed",
	stopped: () => "stopped",
	notStarted: () => "notStarted",
	noPlace: () => "queued",
	unknown: () => "unknown",
};

export interface StageSource {
	readonly runs: FormSessionState["runs"];
	readonly view: Pick<FormSessionState["view"], "selectedRunId">;
}

/**
 * The run on the stage: the selected one, else the newest run that does not wait in line or for
 * its files. Null shows the empty stage (also while the first run's files are sent).
 */
export const stageRunOf = (state: StageSource): RunEntry | null => {
	const { selectedRunId } = state.view;
	const selected =
		selectedRunId === null
			? undefined
			: state.runs.find((run) => run.id === selectedRunId);
	return selected ?? state.runs.find((run) => !WAITING.has(run.status)) ?? null;
};

/** The run's clock: from its first sign of life to its end, or to `now` while it is live; 0 before it started. */
export function elapsedMs(
	entry: Pick<RunEntry, "startedAt" | "endedAt" | "status">,
	now: number,
) {
	if (entry.startedAt === null) return 0;
	const end = entry.endedAt ?? (LIVE.has(entry.status) ? now : entry.startedAt);
	return Math.max(0, end - entry.startedAt);
}

const isRecord = (value: unknown): value is Record<string, unknown> =>
	typeof value === "object" && value !== null && !Array.isArray(value);

const isFileSlot = (value: unknown): value is FileSlot =>
	isRecord(value) &&
	typeof value.id === "string" &&
	typeof value.name === "string" &&
	SLOT_STATES.has(String(value.state)) &&
	"ref" in value &&
	"expiresAt" in value;

function isHiddenValue(value: unknown) {
	return isRecord(value) && value.$hidden === true;
}

/** The value, or one of its object's properties, was kept out of storage (a saved run read back). */
const holdsHidden = (value: CopyValue) =>
	isHiddenValue(value) ||
	(isRecord(value) &&
		!isFileSlot(value) &&
		Object.values(value).some(isHiddenValue));

function slotsOf(value: CopyValue) {
	if (isFileSlot(value)) return [value];
	return Array.isArray(value)
		? (value as readonly unknown[]).filter(isFileSlot)
		: [];
}

/** S4: a file can be sent again while this page holds it (not a reminder) and its upload has not expired. */
function canSendAgain(slot: FileSlot, now: number) {
	return (
		slot.state !== "reminder" &&
		(slot.expiresAt === null || slot.expiresAt > now)
	);
}

/** A run the form left before its turn: its File objects were let go, so none of its files can be sent (M5). */
function closedBeforeItsTurn(entry: Pick<RunEntry, "outcome">) {
	return (
		entry.outcome?.kind === "notStarted" &&
		entry.outcome.reason === "formClosed"
	);
}

/**
 * What keeps "Run again" / "Try again" from repeating the run exactly; files first (the menu names them),
 * then values kept out of storage, then inputs today's form would refuse.
 */
export type RepeatBlocker = "files" | "hidden" | "form";

export type RepeatSource = Pick<RunEntry, "copy" | "outcome">;

const isFileField = (field: WorkbenchField) =>
	field.kind === "file" || field.kind === "files";

/** Today's fields, files aside, still take the copy: a field added since or a changed rule would refuse it at dispatch. */
const fitsForm = (
	fields: readonly WorkbenchField[],
	values: RepeatSource["copy"]["values"],
) =>
	buildPayload(
		fields.filter((field) => !isFileField(field)),
		values,
	).ok;

export const repeatBlocker = (
	entry: RepeatSource,
	now: number,
	fields?: readonly WorkbenchField[],
): RepeatBlocker | null => {
	const closed = closedBeforeItsTurn(entry);
	let hidden = false;
	for (const value of Object.values(entry.copy.values)) {
		if (holdsHidden(value)) hidden = true;
		else if (slotsOf(value).some((slot) => closed || !canSendAgain(slot, now)))
			return "files";
	}
	if (hidden) return "hidden";
	return fields && !fitsForm(fields, entry.copy.values) ? "form" : null;
};

/**
 * S4: the run's own copy can be sent again exactly: every file is still held and not expired,
 * no value (or object property) was kept out of storage and lost to a reload, a run the form left
 * before its turn had no files, and, given today's `fields`, the form still takes its inputs.
 * Otherwise "Use these inputs".
 */
export function repeatable(
	entry: RepeatSource,
	now: number,
	fields?: readonly WorkbenchField[],
) {
	return repeatBlocker(entry, now, fields) === null;
}

/** An answer, files or a result came back; `{ value: false }` and `{ value: null }` count. */
export function hasOutput(output: RunOutput | null) {
	return (
		output !== null &&
		(output.answer.trim() !== "" ||
			output.attachments.length > 0 ||
			output.result !== null)
	);
}

/** The status a settled run takes: a succeeded run without output is `empty`; a refused start queues again. */
export function statusOfOutcome(outcome: RunOutcome, output: RunOutput | null) {
	return STATUS_OF_OUTCOME[outcome.kind](output);
}

/** Questions of the run still waiting for the person ("Waiting for you"); `expires_at` is in seconds. */
export function pendingInteractionsOf(output: RunOutput | null, now: number) {
	if (!output) return [];
	return output.interactions.filter(
		(interaction) =>
			interaction.status === "pending" &&
			!(interaction.expires_at > 0 && interaction.expires_at * 1000 <= now),
	);
}

/** A started run's live status from what it sent: asking while a question waits, streaming once answer text arrives. */
export const liveStatusOf = (
	output: RunOutput | null,
	now: number,
): "asking" | "streaming" | "running" => {
	if (pendingInteractionsOf(output, now).length > 0) return "asking";
	return output && output.answer.trim() !== "" ? "streaming" : "running";
};
