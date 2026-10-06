/*
 * File slots are reference-counted from state (PLAN §3.2). Two kinds of holder:
 * - a slot's File is held by the rail values, next files, left-out files, every run of this session
 *   (ended runs too: "Run again" and "Try again" need it while the page is open) and the one-level
 *   Undo entry;
 * - a slot's upload is needed by the rail values, next files, and runs still waiting (queued,
 *   sending).
 * After every transition the reducer compares before and after: an upload in flight that nothing
 * needs any more is aborted (the slot waits again, so a later holder can send it), and a File the last
 * holder dropped is released. Uploads start at most FORM_LIMITS.uploadsAhead at a time: files waiting
 * runs hold first (oldest run first), then the rail in form order, then next files in list order.
 * Pure.
 */
import {
	type CopyValue,
	FORM_LIMITS,
	type FileSlot,
	type FormSessionState,
	type LeftOutFile,
	type RunEntry,
	type UndoEntry,
	type WorkbenchField,
} from "../contracts";
import { isFileField, isFileSlot, isReminder, slotsOf } from "../model/values";
import { type Tx, emit } from "./reduce-tx";
import { isWaiting } from "./state";

type SlotUpdate = (slot: FileSlot) => FileSlot;
type SlotLists = Readonly<Record<string, readonly FileSlot[]>>;

/** A slot with a File behind it ("Pick again" reminders have none). */
const hasFile = (slot: FileSlot) => !isReminder(slot);

const fileFieldsOf = (state: Pick<FormSessionState, "form">) =>
	state.form.fields.filter(isFileField);

function addIds(ids: Set<string>, value: unknown) {
	for (const slot of slotsOf(value)) if (hasFile(slot)) ids.add(slot.id);
}

function addHeld(
	ids: Set<string>,
	state: FormSessionState,
	field: WorkbenchField,
) {
	const { rail, undo } = state;
	addIds(ids, rail.values[field.name]);
	addIds(ids, rail.nextFiles[field.name]);
	for (const left of rail.leftOut[field.name] ?? []) addIds(ids, left.slot);
	for (const run of state.runs)
		if (run.origin === "session") addIds(ids, run.copy.values[field.name]);
	if (undo) {
		addIds(ids, undo.values[field.name]);
		addIds(ids, undo.nextFiles[field.name]);
	}
}

/** Every slot whose File something in the state still holds. */
export function slotHolders(state: FormSessionState): ReadonlySet<string> {
	const ids = new Set<string>();
	for (const field of fileFieldsOf(state)) addHeld(ids, state, field);
	return ids;
}

export interface NeededSlot {
	readonly slot: FileSlot;
	/** The field the slot sits in: its name and FileMode go with the upload. */
	readonly field: WorkbenchField;
}

const byCreation = (a: RunEntry, b: RunEntry) =>
	a.createdAt - b.createdAt || a.n - b.n;

/** Slots whose upload something still needs, in upload order. */
export function uploadNeeds(
	state: FormSessionState,
): ReadonlyMap<string, NeededSlot> {
	const needs = new Map<string, NeededSlot>();
	const fields = fileFieldsOf(state);
	const add = (field: WorkbenchField, value: unknown) => {
		for (const slot of slotsOf(value))
			if (hasFile(slot) && !needs.has(slot.id))
				needs.set(slot.id, { slot, field });
	};
	const waiting = state.runs
		.filter((run) => run.origin === "session" && isWaiting(run))
		.sort(byCreation);
	for (const run of waiting)
		for (const field of fields) add(field, run.copy.values[field.name]);
	for (const field of fields) add(field, state.rail.values[field.name]);
	for (const field of fields) add(field, state.rail.nextFiles[field.name]);
	return needs;
}

// ─── Updating a slot wherever it is ─────────────────────────────────────────

function mapValue<V>(value: V, update: SlotUpdate): V {
	if (isFileSlot(value)) return update(value) as V;
	if (!Array.isArray(value) || !value.some(isFileSlot)) return value;
	let changed = false;
	const list = value.map((item) => {
		if (!isFileSlot(item)) return item;
		const next = update(item);
		changed = changed || next !== item;
		return next;
	});
	return (changed ? list : value) as V;
}

function mapRecord<V>(
	record: Readonly<Record<string, V>>,
	update: SlotUpdate,
): Readonly<Record<string, V>> {
	let changed = false;
	const out: Record<string, V> = {};
	for (const [name, value] of Object.entries(record)) {
		const next = mapValue(value, update);
		changed = changed || next !== value;
		out[name] = next;
	}
	return changed ? out : record;
}

function mapLeftOut(
	record: Readonly<Record<string, readonly LeftOutFile[]>>,
	update: SlotUpdate,
) {
	const mapList = (list: readonly LeftOutFile[]) => {
		const next = list.map((item) => {
			const slot = update(item.slot);
			return slot === item.slot ? item : { ...item, slot };
		});
		return next.some((item, index) => item !== list[index]) ? next : list;
	};
	let changed = false;
	const out: Record<string, readonly LeftOutFile[]> = {};
	for (const [name, list] of Object.entries(record)) {
		out[name] = mapList(list);
		changed = changed || out[name] !== list;
	}
	return changed ? out : record;
}

function mapRuns(runs: readonly RunEntry[], update: SlotUpdate) {
	let changed = false;
	const next = runs.map((run) => {
		if (run.origin !== "session") return run;
		const values = mapRecord<CopyValue>(run.copy.values, update);
		if (values === run.copy.values) return run;
		changed = true;
		return { ...run, copy: { ...run.copy, values } };
	});
	return changed ? next : runs;
}

function mapUndo(undo: UndoEntry | null, update: SlotUpdate) {
	if (!undo) return undo;
	const values = mapRecord(undo.values, update);
	const nextFiles = mapRecord<readonly FileSlot[]>(undo.nextFiles, update);
	if (values === undo.values && nextFiles === undo.nextFiles) return undo;
	return { ...undo, values, nextFiles };
}

/** Applies `update` to every slot in the state; unchanged parts keep their identity. */
export function mapSlots(
	state: FormSessionState,
	update: SlotUpdate,
): FormSessionState {
	const { rail } = state;
	const values = mapRecord(rail.values, update);
	const nextFiles = mapRecord<readonly FileSlot[]>(rail.nextFiles, update);
	const leftOut = mapLeftOut(rail.leftOut, update);
	const runs = mapRuns(state.runs, update);
	const undo = mapUndo(state.undo, update);
	const railChanged =
		values !== rail.values ||
		nextFiles !== rail.nextFiles ||
		leftOut !== rail.leftOut;
	if (!railChanged && runs === state.runs && undo === state.undo) return state;
	return {
		...state,
		rail: railChanged ? { ...rail, values, nextFiles, leftOut } : rail,
		runs,
		undo,
	};
}

/** Updates one slot by id wherever it appears. */
export function updateSlot(
	state: FormSessionState,
	id: string,
	update: SlotUpdate,
): FormSessionState {
	return mapSlots(state, (slot) => (slot.id === id ? update(slot) : slot));
}

/** Whether a slot's upload is in flight now (a sending slot is always needed: unneeded ones are aborted). */
export const isUploading = (state: FormSessionState, id: string) =>
	uploadNeeds(state).get(id)?.slot.state === "sending";

// ─── After every transition ─────────────────────────────────────────────────

const waitAgain: SlotUpdate = (slot) =>
	slot.state === "sending"
		? { ...slot, state: "waiting", progress: null }
		: slot;

/** Uploads in flight before the step that nothing needs after it (and those the step aborted itself). */
function abortedBetween(
	before: FormSessionState,
	after: FormSessionState,
	tx: Tx,
) {
	const needed = uploadNeeds(after);
	const aborted = new Set(tx.aborted);
	for (const [id, need] of uploadNeeds(before))
		if (need.slot.state === "sending" && !needed.has(id)) aborted.add(id);
	return aborted;
}

/** Files held before the step (or picked in it) that nothing holds after it. */
function releasedBetween(
	before: FormSessionState,
	after: FormSessionState,
	tx: Tx,
) {
	const held = slotHolders(after);
	const released = new Set(tx.released);
	for (const id of [...slotHolders(before), ...tx.picked])
		if (!held.has(id)) released.add(id);
	return released;
}

/**
 * Aborts uploads nothing needs any more and releases Files the last holder dropped. Slots picked
 * in this step count as held before it (a refused pick is released at once).
 */
export function settleSlots(
	before: FormSessionState,
	after: FormSessionState,
	tx: Tx,
) {
	const aborted = abortedBetween(before, after, tx);
	const released = releasedBetween(before, after, tx);
	for (const slotId of aborted) emit(tx, { type: "abortUpload", slotId });
	if (released.size > 0)
		emit(tx, { type: "releaseFiles", slotIds: [...released] });
	if (aborted.size === 0) return after;
	return mapSlots(after, (slot) =>
		aborted.has(slot.id) ? waitAgain(slot) : slot,
	);
}

/** Waiting uploads to start now, in upload order, until FORM_LIMITS.uploadsAhead are in flight. */
function uploadsToStart(needs: readonly NeededSlot[]) {
	let inFlight = needs.filter((need) => need.slot.state === "sending").length;
	const starting: NeededSlot[] = [];
	for (const need of needs) {
		if (inFlight >= FORM_LIMITS.uploadsAhead) break;
		if (need.slot.state !== "waiting") continue;
		starting.push(need);
		inFlight += 1;
	}
	return starting;
}

/** Starts uploads of needed slots that wait, at most FORM_LIMITS.uploadsAhead in flight. */
export function scheduleUploads(state: FormSessionState, tx: Tx) {
	if (state.form.host.uploads !== "temporary") return state;
	const starting = uploadsToStart([...uploadNeeds(state).values()]);
	if (starting.length === 0) return state;
	for (const need of starting)
		emit(tx, {
			type: "upload",
			slotId: need.slot.id,
			name: need.field.name,
			mode: need.field.fileMode ?? "url",
		});
	const ids = new Set(starting.map((need) => need.slot.id));
	return mapSlots(state, (slot) =>
		ids.has(slot.id) ? { ...slot, state: "sending", progress: 0 } : slot,
	);
}

/** Slot ids a run must wait for: its files not sent yet. */
export function pendingSlotIds(
	fields: readonly WorkbenchField[],
	values: Readonly<Record<string, CopyValue>>,
): readonly string[] {
	return fields
		.filter(isFileField)
		.flatMap((field) => slotsOf(values[field.name]))
		.filter((slot) => slot.state === "waiting" || slot.state === "sending")
		.map((slot) => slot.id);
}
