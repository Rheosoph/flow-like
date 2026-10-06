/*
 * What this device remembers (spec §5, M6, S1, S4): loading it (saved runs merged after this
 * session's, the next run number past every saved run, the on-open preset applied silently, a series
 * never survives a reload), forgetting a recent value, "Don't save {label}", "Use these inputs" (files
 * that can no longer be sent come back as "Pick again", kept-out values must be entered again),
 * removing a run from this device, and the one-level Undo.
 */
import {
	type CopyValue,
	type FieldKey,
	type FieldValue,
	type FieldValues,
	type FileSlot,
	type FormPrefs,
	type FormSessionState,
	LIVE_RUN_STATUSES,
	type RunEntry,
	type UndoEntry,
	type WorkbenchField,
} from "../contracts";
import { hashValue } from "../model/secrets";
import { fromStoredInputs } from "../model/stored";
import {
	HIDDEN_VALUE,
	baselineOf,
	emptyValue,
	groupOf,
	isFileField,
	isFileSlot,
	isHiddenValue,
	isReminder,
	slotsOf,
} from "../model/values";
import { mergeHistory, nextNumberAfter } from "./history";
import { applyPresetValues, restorePreset } from "./reduce-presets";
import {
	type CommandHandlers,
	type InputHandlers,
	type Tx,
	clearEdits,
	emit,
	fieldFocus,
	withFocus,
	withMessage,
	withPrefs,
	withRail,
	withUndoMessage,
	withView,
} from "./reduce-tx";
import { withListInRange } from "./reduce-view";
import { activePreset, isSplit, runById, targetOf } from "./state";

const DAY_MS = 24 * 60 * 60 * 1000;

/** Every field the form has now is stamped as seen (names unseen for 90 days are pruned by the store). */
function stampedSeenAt(
	seenAt: FormPrefs["fieldSeenAt"],
	fields: readonly WorkbenchField[],
	now: number,
) {
	const stale = fields.filter(
		(field) => (seenAt[field.name] ?? Number.NEGATIVE_INFINITY) < now - DAY_MS,
	);
	if (stale.length === 0) return seenAt;
	return {
		...seenAt,
		...Object.fromEntries(stale.map((field) => [field.name, now])),
	};
}

/** The preset marked "Apply when this form opens", applied without a message (its name on the button says it). */
function applyOnOpen(state: FormSessionState): FormSessionState {
	if (state.rail.activePresetId !== null) return state;
	const preset = state.memory.presets.find((item) => item.openDefault);
	if (!preset) return state;
	return withRail(state, {
		values: applyPresetValues(state, preset),
		activePresetId: preset.id,
	});
}

/** Memory arrived: history merged, run numbers continue past every run, a stale series is dropped. */
const memoryLoaded: InputHandlers<"memoryLoaded">["memoryLoaded"] = (
	state,
	input,
	tx,
) => {
	const { memory } = input;
	const loaded = memory.prefs ?? state.memory.prefs;
	const runs = mergeHistory(state.runs, memory.runs, state.form);
	const prefs: FormPrefs = {
		...loaded,
		auto: [],
		nextRunNumber: Math.max(
			loaded.nextRunNumber,
			state.memory.prefs.nextRunNumber,
			nextNumberAfter(runs),
		),
		fieldSeenAt: stampedSeenAt(
			loaded.fieldSeenAt,
			state.form.fields,
			tx.clock.now,
		),
	};
	const changed =
		memory.prefs === null ||
		prefs.nextRunNumber !== loaded.nextRunNumber ||
		prefs.fieldSeenAt !== loaded.fieldSeenAt ||
		loaded.auto.length > 0;
	if (changed) emit(tx, { type: "persistPrefs", prefs });
	const next: FormSessionState = {
		...state,
		runs,
		memory: { loaded: true, prefs, presets: memory.presets },
	};
	return applyOnOpen(next);
};

export const MEMORY_INPUTS: InputHandlers<"memoryLoaded"> = { memoryLoaded };

// ─── Recent values and secrets ──────────────────────────────────────────────

/**
 * Delete on a row of the recent list: its hash is saved, so it is not offered again (the runs keep
 * it). The list's active row stays inside the rows that are left.
 */
const forgetRecent: CommandHandlers<"forgetRecent">["forgetRecent"] = (
	state,
	command,
	tx,
) => {
	const { forgotten } = state.memory.prefs;
	const hash = hashValue(command.value);
	const list = forgotten[command.name] ?? [];
	if (list.includes(hash)) return state;
	const saved = withPrefs(state, tx, {
		forgotten: { ...forgotten, [command.name]: [...list, hash] },
	});
	return withListInRange(saved);
};

/**
 * A saved run's value at a key, kept out as a reload reads it back: the whole value, or one property
 * marked `{ $hidden: true }`, so Run again refuses the copy and "Use these inputs" asks for it again.
 */
function hiddenIn(
	run: RunEntry,
	key: FieldKey,
	state: FormSessionState,
): RunEntry {
	const target = targetOf(state, key);
	if (!target) return run;
	const name = target.group ?? target.field.name;
	const value = run.copy.values[name];
	if (value === undefined || isHiddenValue(value)) return run;
	const hidden: CopyValue =
		target.group === null
			? HIDDEN_VALUE
			: { ...groupOf(value), [target.field.name]: { $hidden: true } };
	return {
		...run,
		copy: { ...run.copy, values: { ...run.copy.values, [name]: hidden } },
	};
}

/**
 * "Don't save {label} on this device": saved runs lose the value (here and in storage); this
 * session's runs keep it in memory for "Run again" until the page closes.
 */
const dontSave: CommandHandlers<"dontSave">["dontSave"] = (
	state,
	command,
	tx,
) => {
	const target = targetOf(state, command.name);
	const { noSave } = state.memory.prefs;
	if (!target || noSave.includes(command.name)) return state;
	emit(tx, { type: "hideField", name: command.name });
	const saved = withPrefs(state, tx, { noSave: [...noSave, command.name] });
	const runs = saved.runs.map((run) =>
		run.origin === "history" ? hiddenIn(run, command.name, saved) : run,
	);
	const list = saved.view.list?.key === command.name ? null : saved.view.list;
	return withMessage(withView({ ...saved, runs }, { list }), tx, {
		kind: "noSave",
		label: target.field.label,
	});
};

// ─── Use these inputs (S4) ──────────────────────────────────────────────────

/** A file of this session that expired cannot be sent again: it must be picked again. */
function expiredAsReminder(item: unknown, now: number) {
	if (!isFileSlot(item) || isReminder(item)) return item;
	if (item.expiresAt === null || item.expiresAt > now) return item;
	return { ...item, state: "reminder" as const, ref: null, progress: null };
}

function railValueOf(
	field: WorkbenchField,
	value: CopyValue | undefined,
	start: FieldValue,
	now: number,
): FieldValue {
	if (value === undefined) return start;
	if (isHiddenValue(value)) return emptyValue(field);
	if (Array.isArray(value))
		return value.map((item) => expiredAsReminder(item, now)) as FieldValue;
	return expiredAsReminder(value, now) as FieldValue;
}

/** While a series runs, the field's current file goes back to the front of its next files (M3). */
function currentBack(
	state: FormSessionState,
	values: FieldValues,
): FormSessionState["rail"]["nextFiles"] {
	const { rail } = state;
	const out: Record<string, readonly FileSlot[]> = { ...rail.nextFiles };
	for (const [name, list] of Object.entries(rail.nextFiles)) {
		const current = slotsOf(rail.values[name]).find(
			(slot) => !isReminder(slot),
		);
		const incoming = slotsOf(values[name])[0];
		if (current && current.id !== incoming?.id) out[name] = [current, ...list];
	}
	return out;
}

const reminders = (fields: readonly WorkbenchField[], values: FieldValues) =>
	fields
		.filter(isFileField)
		.flatMap((field) => slotsOf(values[field.name]))
		.filter(isReminder).length;

/** "Run 9's inputs are in. Pick 3 files again." with Undo; the rail shows Inputs, the cursor goes to Run. */
const useInputs: CommandHandlers<"useInputs">["useInputs"] = (
	state,
	command,
	tx,
) => {
	const run = runById(state, command.runId);
	const { fields } = state.form;
	if (!run || fields.length === 0) return state;
	const read = fromStoredInputs(fields, run.copy.values);
	const base = baselineOf(fields, activePreset(state));
	const values: FieldValues = Object.fromEntries(
		fields.map((field) => [
			field.name,
			railValueOf(
				field,
				read.values[field.name],
				base[field.name],
				tx.clock.now,
			),
		]),
	);
	const filled = withRail(clearEdits(state), {
		values,
		nextFiles: currentBack(state, values),
		texts: {},
		problems: {},
		pressed: false,
		entryBegan: {},
		tab: "inputs",
		comparedRunId: run.id,
	});
	const shown = isSplit(filled) ? filled : withView(filled, { pane: "inputs" });
	const enterAgain = read.enterAgain.map(
		(key) => targetOf(state, key)?.field.label ?? key,
	);
	const told = withUndoMessage(
		shown,
		tx,
		{
			kind: "inputsIn",
			n: run.n,
			pickAgain: reminders(fields, values),
			enterAgain,
			misfit: read.misfit,
		},
		{ kind: "useInputs", before: state },
	);
	return withFocus(told, tx, { kind: "run" });
};

// ─── Removing a run, Undo ───────────────────────────────────────────────────

const LIVE: ReadonlySet<string> = new Set(LIVE_RUN_STATUSES);

/** "Remove from this device": an ended run goes from the lists and from storage. */
const removeRun: CommandHandlers<"removeRun">["removeRun"] = (
	state,
	command,
	tx,
) => {
	const run = runById(state, command.runId);
	if (!run || LIVE.has(run.status)) return state;
	emit(tx, { type: "deleteRun", id: run.id });
	const { view, rail } = state;
	const selected = view.selectedRunId === run.id;
	const menu = view.overlay?.id === "runMenu" && view.overlay.runId === run.id;
	const next = withView(
		{ ...state, runs: state.runs.filter((item) => item.id !== run.id) },
		{
			selectedRunId: selected ? null : view.selectedRunId,
			stageFollowsNewest: selected ? true : view.stageFollowsNewest,
			pinnedRunId: view.pinnedRunId === run.id ? null : view.pinnedRunId,
			overlay: menu ? null : view.overlay,
		},
	);
	return rail.comparedRunId === run.id
		? withRail(next, { comparedRunId: null })
		: next;
};

/**
 * Undoing a file removal draws the file row again in place of the drop row that had the cursor: the
 * cursor goes to the field whose file came back, so it never falls to the page.
 */
function focusRestoredFile(
	state: FormSessionState,
	before: FieldValues,
	entry: UndoEntry,
	tx: Tx,
): FormSessionState {
	if (entry.kind !== "fileRemoved") return state;
	const field = state.form.fields.find(
		(candidate) =>
			isFileField(candidate) &&
			before[candidate.name] !== entry.values[candidate.name],
	);
	return field ? withFocus(state, tx, fieldFocus(field.key)) : state;
}

/** ⌘Z or the dock's "Undo": the values, next files and preset as they were before the reported change. */
const undo: CommandHandlers<"undo">["undo"] = (state, _command, tx) => {
	const entry = state.undo;
	if (!entry) return state;
	const restored = withRail(state, {
		values: entry.values,
		nextFiles: entry.nextFiles,
		activePresetId: entry.activePresetId,
		problems: {},
		texts: {},
	});
	const presets = restorePreset(restored, tx, entry.kind, entry.deletedPreset);
	const message = presets.view.message?.undo ? null : presets.view.message;
	const undone = { ...withView(presets, { message }), undo: null };
	return focusRestoredFile(undone, state.rail.values, entry, tx);
};

export const MEMORY_COMMANDS: CommandHandlers<
	"forgetRecent" | "dontSave" | "useInputs" | "removeRun" | "undo"
> = {
	forgetRecent,
	dontSave,
	useInputs,
	removeRun,
	undo,
};
