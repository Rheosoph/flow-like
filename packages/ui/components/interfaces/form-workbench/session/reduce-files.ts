/*
 * Files (spec F, M3): a pick fills the field (several files for a one-file field queue as next files,
 * files a run on this device already had are left out, the field becomes per run for the series),
 * ⌫ on the current file pulls the next one in, next files can be removed one by one or all at once,
 * left-out files can be added back, and inline hosts refuse or warn by size. Upload results update a
 * slot wherever it is and move sending runs on.
 */
import {
	type DockMessage,
	FORM_LIMITS,
	type FileSlot,
	type FormSessionState,
	type HostCapabilities,
	type LeftOutFile,
	type PickedFile,
	type WorkbenchField,
} from "../contracts";
import {
	type InlineCheck,
	inlineCheck,
	naturalCompare,
	queueFiles,
	sameFile,
	sentFiles,
} from "../model/next-files";
import { autoPerRun } from "../model/per-run";
import { isBlocked } from "../model/validate";
import { isReminder, slotsOf } from "../model/values";
import { endSeries } from "./reduce-per-run";
import { notStarted } from "./reduce-runs";
import {
	type CommandHandlers,
	type InputHandlers,
	type Tx,
	changeRun,
	clearEdits,
	fieldFocus,
	withFocus,
	withMessage,
	withPrefs,
	withRail,
	withUndoMessage,
	withView,
	withoutProblem,
} from "./reduce-tx";
import { isUploading, updateSlot } from "./slots";
import { fieldByName, livePerRun } from "./state";

type Lists = Readonly<Record<string, readonly FileSlot[]>>;

const byName = (a: FileSlot, b: FileSlot) => naturalCompare(a.name, b.name);

function withoutName<T>(record: Readonly<Record<string, T>>, name: string) {
	if (!(name in record)) return record;
	const { [name]: _gone, ...rest } = record;
	return rest;
}

/** The field's current file, if it holds a real one (not a "Pick again" reminder). */
const currentFile = (state: FormSessionState, name: string) =>
	slotsOf(state.rail.values[name]).find((slot) => !isReminder(slot)) ?? null;

function newSlot(
	file: PickedFile,
	host: HostCapabilities,
	now: number,
): FileSlot {
	const inline = host.uploads === "inline";
	return {
		id: file.slotId,
		name: file.name,
		size: file.size,
		type: file.type || null,
		state: inline ? "sent" : "waiting",
		progress: null,
		ref: inline ? { kind: "inline" } : null,
		error: null,
		sentAt: inline ? now : null,
		expiresAt: null,
	};
}

/** Runs that were given their files: every run but those taken out before they began. */
const sentSources = (state: FormSessionState) =>
	state.runs
		.filter((run) => run.status !== "notStarted")
		.map((run) => ({ n: run.n, values: run.copy.values }));

// ─── Picking ────────────────────────────────────────────────────────────────

interface PickResult {
	readonly state: FormSessionState;
	readonly message: DockMessage | null;
}

/** A multi-file pick makes the field per run for its series; with nothing per run yet, `autoPerRun` too. */
function startSeries(
	state: FormSessionState,
	name: string,
	tx: Tx,
): FormSessionState {
	const prefs = state.memory.prefs;
	const current = livePerRun(state);
	if (current.includes(name)) return state;
	const auto =
		current.length === 0
			? autoPerRun(state.form.fields, name, state.rail.values)
			: [...prefs.auto, name];
	return withPrefs(state, tx, { auto, introduced: true });
}

function queuePick(
	state: FormSessionState,
	field: WorkbenchField,
	slots: readonly FileSlot[],
	tx: Tx,
): PickResult {
	const { rail } = state;
	const queued = queueFiles(
		currentFile(state, field.name),
		rail.nextFiles[field.name] ?? [],
		slots,
		sentFiles(sentSources(state), field.name),
	);
	const series = queued.current !== null;
	const next = withRail(state, {
		values: { ...rail.values, [field.name]: queued.current },
		nextFiles: series
			? { ...rail.nextFiles, [field.name]: queued.next }
			: withoutName(rail.nextFiles, field.name),
		leftOut:
			queued.leftOut.length > 0
				? { ...rail.leftOut, [field.name]: queued.leftOut }
				: withoutName(rail.leftOut, field.name),
	});
	const listed = queued.next.length > 0;
	const opened = listed
		? withView(next, {
				list: { kind: "nextFiles", key: field.name, active: -1 },
			})
		: next;
	return {
		state: series ? startSeries(opened, field.name, tx) : opened,
		message: pickMessage(queued.dropped, listed, queued.leftOut),
	};
}

function pickMessage(
	dropped: number,
	listed: boolean,
	leftOut: readonly LeftOutFile[],
): DockMessage | null {
	if (dropped > 0) return { kind: "nextFilesCapped" };
	if (!listed && leftOut.length > 0) return { kind: "leftOut", files: leftOut };
	return null;
}

/** One picked file replaces the current one; a hosted link keeps the first of several (by name). */
function pickOne(
	state: FormSessionState,
	field: WorkbenchField,
	slots: readonly FileSlot[],
	tx: Tx,
): PickResult {
	if (slots.length > 1 && state.form.host.nextFiles)
		return queuePick(state, field, slots, tx);
	const [first] = [...slots].sort(byName);
	const values = { ...state.rail.values, [field.name]: first };
	const message: DockMessage | null =
		slots.length > 1 ? { kind: "oneFileOnly", name: first.name } : null;
	return { state: withRail(state, { values }), message };
}

/** A several-files field: the files join (or replace) the field's files; a same-named reminder goes. */
function pickMany(
	state: FormSessionState,
	field: WorkbenchField,
	slots: readonly FileSlot[],
	append: boolean,
): PickResult {
	const held = append ? slotsOf(state.rail.values[field.name]) : [];
	const kept = held.filter(
		(slot) =>
			!(isReminder(slot) && slots.some((picked) => picked.name === slot.name)),
	);
	const fresh = slots.filter(
		(picked) =>
			!kept.some((slot) => !isReminder(slot) && sameFile(slot, picked)),
	);
	const values = { ...state.rail.values, [field.name]: [...kept, ...fresh] };
	return { state: withRail(state, { values }), message: null };
}

function refusedMessage(
	check: InlineCheck<PickedFile>,
	host: HostCapabilities,
): DockMessage | null {
	const refused = check.refused[0];
	if (!refused) return null;
	return {
		kind: "fileRefused",
		name: refused.name,
		limitBytes: refused.limitBytes,
		host: host.kind,
	};
}

function warnedMessage(check: InlineCheck<PickedFile>): DockMessage | null {
	const warned = check.warned[0];
	return warned ? { kind: "fileWarning", name: warned.name } : null;
}

/** "Add it" stays until the next edit or run; the other pick messages are plain. */
const lifeOf = (message: DockMessage) =>
	message.kind === "leftOut" ? "action" : "plain";

function placePick(
	state: FormSessionState,
	field: WorkbenchField,
	slots: readonly FileSlot[],
	append: boolean,
	tx: Tx,
): PickResult {
	if (slots.length === 0) return { state, message: null };
	return field.kind === "file"
		? pickOne(state, field, slots, tx)
		: pickMany(state, field, slots, append);
}

/** A file field this host can fill; a FlowPath field it cannot fill takes no file. */
function pickTarget(state: FormSessionState, name: string) {
	const field = fieldByName(state, name);
	if (!field || (field.kind !== "file" && field.kind !== "files")) return null;
	return isBlocked(field, state.form.host) ? null : field;
}

function told(state: FormSessionState, tx: Tx, message: DockMessage | null) {
	return message ? withMessage(state, tx, message, lifeOf(message)) : state;
}

/**
 * Files picked or dropped for a field. The cursor goes to the field (its attached row). Messages,
 * highest first: a refused file, the pick's own (cap, one file only, left out), a size warning.
 * Files a host refuses are released at once.
 */
const pickFiles: CommandHandlers<"pickFiles">["pickFiles"] = (
	state,
	command,
	tx,
) => {
	tx.picked.push(...command.files.map((file) => file.slotId));
	const field = pickTarget(state, command.name);
	if (!field) return state;
	const { host } = state.form;
	const check = inlineCheck(host, command.files);
	if (check.added.length === 0 && check.refused.length === 0) return state;
	const slots = check.added.map((file) => newSlot(file, host, tx.clock.now));
	const cleared = withoutProblem(clearEdits(state), field.key);
	const append = command.mode === "append";
	const picked = placePick(cleared, field, slots, append, tx);
	const message =
		refusedMessage(check, host) ?? picked.message ?? warnedMessage(check);
	return withFocus(told(picked.state, tx, message), tx, fieldFocus(field.key));
};

// ─── Removing ───────────────────────────────────────────────────────────────

/** The current file goes: the next file moves in; with none waiting the series is over. */
function pullNext(state: FormSessionState, name: string): FormSessionState {
	const { rail } = state;
	const waiting = rail.nextFiles[name];
	if (waiting && waiting.length > 0)
		return withRail(state, {
			values: { ...rail.values, [name]: waiting[0] },
			nextFiles: { ...rail.nextFiles, [name]: waiting.slice(1) },
		});
	const emptied = withRail(state, {
		values: { ...rail.values, [name]: null },
		nextFiles: withoutName(rail.nextFiles, name),
	});
	return waiting ? endSeries(emptied) : emptied;
}

/** ⌫ on an attached row (or a reminder): "{name} removed." with Undo; focus stays on the field. */
export function removeSlotFrom(
	state: FormSessionState,
	field: WorkbenchField,
	slotId: string,
	tx: Tx,
): FormSessionState {
	const slots = slotsOf(state.rail.values[field.name]);
	const slot = slots.find((candidate) => candidate.id === slotId);
	if (!slot) return state;
	const cleared = clearEdits(state);
	const removed =
		field.kind === "files"
			? withRail(cleared, {
					values: {
						...cleared.rail.values,
						[field.name]: slots.filter((item) => item.id !== slotId),
					},
				})
			: pullNext(cleared, field.name);
	const next = withUndoMessage(
		withoutProblem(removed, field.key),
		tx,
		{ kind: "fileRemoved", name: slot.name },
		{ kind: "fileRemoved", before: state },
	);
	return withFocus(next, tx, fieldFocus(field.key));
}

const removeFile: CommandHandlers<"removeFile">["removeFile"] = (
	state,
	command,
	tx,
) => {
	const field = fieldByName(state, command.name);
	return field ? removeSlotFrom(state, field, command.slotId, tx) : state;
};

/** The list's active row stays inside the list. */
function clampList(state: FormSessionState, name: string): FormSessionState {
	const list = state.view.list;
	if (list?.kind !== "nextFiles" || list.key !== name) return state;
	const rows = state.rail.nextFiles[name]?.length ?? 0;
	if (rows === 0) return withView(state, { list: null });
	return withView(state, {
		list: { ...list, active: Math.min(list.active, rows - 1) },
	});
}

/** Next files of a field after some were removed: an empty list keeps the current file as the series' last. */
function withNextFiles(
	state: FormSessionState,
	name: string,
	files: readonly FileSlot[],
): FormSessionState {
	const { rail } = state;
	if (files.length > 0 || currentFile(state, name))
		return withRail(state, { nextFiles: { ...rail.nextFiles, [name]: files } });
	return endSeries(
		withRail(state, { nextFiles: withoutName(rail.nextFiles, name) }),
	);
}

const removeNextFile: CommandHandlers<"removeNextFile">["removeNextFile"] = (
	state,
	command,
) => {
	const list = state.rail.nextFiles[command.name];
	if (!list?.some((slot) => slot.id === command.slotId)) return state;
	const rest = list.filter((slot) => slot.id !== command.slotId);
	return clampList(
		withNextFiles(clearEdits(state), command.name, rest),
		command.name,
	);
};

/** "Remove all next files": "9 next files removed." with Undo, which puts them back. */
const clearNextFiles: CommandHandlers<"clearNextFiles">["clearNextFiles"] = (
	state,
	command,
	tx,
) => {
	const count = state.rail.nextFiles[command.name]?.length ?? 0;
	if (count === 0) return state;
	const cleared = clampList(
		withNextFiles(clearEdits(state), command.name, []),
		command.name,
	);
	return withUndoMessage(
		cleared,
		tx,
		{ kind: "nextFilesRemoved", count },
		{ kind: "nextFilesRemoved", before: state },
	);
};

/**
 * "Add it" / "Add them": the left-out files join the next files in name order. The current file
 * stays where it is, so a date typed for it never pairs with another file.
 */
const addLeftOut: CommandHandlers<"addLeftOut">["addLeftOut"] = (
	state,
	command,
	tx,
) => {
	const left = state.rail.leftOut[command.name];
	if (!left || left.length === 0) return state;
	const cleared = clearEdits(state);
	const { rail } = cleared;
	const merged = [
		...(rail.nextFiles[command.name] ?? []),
		...left.map((item) => item.slot),
	].sort(byName);
	const current = currentFile(cleared, command.name);
	const head = current ?? merged[0];
	const next = (current ? merged : merged.slice(1)).slice(
		0,
		FORM_LIMITS.nextFiles,
	);
	const added = withRail(cleared, {
		values: { ...rail.values, [command.name]: head },
		nextFiles: { ...rail.nextFiles, [command.name]: next },
		leftOut: withoutName(rail.leftOut, command.name),
	});
	return startSeries(added, command.name, tx);
};

/** "Try again" on a file that was not sent: it waits for its upload again. */
const retryFile: CommandHandlers<"retryFile">["retryFile"] = (
	state,
	command,
) => {
	const retried = updateSlot(state, command.slotId, (slot) =>
		slot.state === "failed"
			? { ...slot, state: "waiting", error: null, progress: null }
			: slot,
	);
	const holder = state.form.fields.find((field) =>
		slotsOf(state.rail.values[field.name]).some(
			(slot) => slot.id === command.slotId,
		),
	);
	return holder ? withoutProblem(retried, holder.key) : retried;
};

export const FILE_COMMANDS: CommandHandlers<
	| "pickFiles"
	| "removeFile"
	| "retryFile"
	| "removeNextFile"
	| "clearNextFiles"
	| "addLeftOut"
> = {
	pickFiles,
	removeFile,
	retryFile,
	removeNextFile,
	clearNextFiles,
	addLeftOut,
};

// ─── Upload results ─────────────────────────────────────────────────────────

const uploadProgress: InputHandlers<"uploadProgress">["uploadProgress"] = (
	state,
	input,
) =>
	updateSlot(state, input.slotId, (slot) =>
		slot.state === "sending"
			? { ...slot, progress: Math.min(1, Math.max(0, input.progress)) }
			: slot,
	);

/** A sent file: sending runs that waited for it join the queue once all their files are sent. */
const uploadSent: InputHandlers<"uploadSent">["uploadSent"] = (
	state,
	input,
	tx,
) => {
	const sent = updateSlot(state, input.slotId, (slot) =>
		isReminder(slot)
			? slot
			: {
					...slot,
					state: "sent",
					ref: input.ref,
					expiresAt: input.expiresAt,
					sentAt: tx.clock.now,
					progress: null,
					error: null,
				},
	);
	return waitingRunsOf(sent, input.slotId).reduce(
		(current, id) =>
			changeRun(current, tx, id, (run) => {
				const pending = run.pendingSlotIds.filter(
					(slot) => slot !== input.slotId,
				);
				return {
					...run,
					pendingSlotIds: pending,
					status: pending.length === 0 ? "queued" : run.status,
				};
			}),
		sent,
	);
};

const waitingRunsOf = (state: FormSessionState, slotId: string) =>
	state.runs
		.filter(
			(run) =>
				run.origin === "session" &&
				run.status === "sending" &&
				run.pendingSlotIds.includes(slotId),
		)
		.map((run) => run.id);

/** A failed upload: the field shows "Not sent · Try again"; a run waiting for the file does not start. */
const uploadFailed: InputHandlers<"uploadFailed">["uploadFailed"] = (
	state,
	input,
	tx,
) => {
	if (!isUploading(state, input.slotId)) return state;
	const failed = updateSlot(state, input.slotId, (slot) => ({
		...slot,
		state: "failed",
		error: input.error,
		progress: null,
	}));
	return waitingRunsOf(failed, input.slotId).reduce(
		(current, id) => notStarted(current, tx, id, "fileNotSent"),
		failed,
	);
};

export const FILE_INPUTS: InputHandlers<
	"uploadProgress" | "uploadSent" | "uploadFailed"
> = {
	uploadProgress,
	uploadSent,
	uploadFailed,
};
