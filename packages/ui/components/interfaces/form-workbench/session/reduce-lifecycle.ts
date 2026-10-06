/*
 * The form's life on the page (PLAN §3.7): leaving it (after the grace without a new mount) turns
 * queued and sending runs into "Not started · the form was closed before its turn", aborts uploads,
 * releases every File and drops next files; a series ends unanswered, so its fields are no longer per
 * run. A changed event or config rebases the values by name; the runs stay.
 */
import type {
	FieldValues,
	FileSlot,
	FormModel,
	FormSessionState,
} from "../contracts";
import { targets } from "../model/fields";
import { capOf } from "../model/queue";
import { fitValue, startingValue } from "../model/values";
import { notStarted } from "./reduce-runs";
import {
	type InputHandlers,
	type Tx,
	withPrefs,
	withRail,
	withView,
} from "./reduce-tx";
import { mapSlots, slotHolders, uploadNeeds } from "./slots";
import { activePreset, isWaiting } from "./state";

/** A file whose upload can still be sent without its File: a FlowPath or a URL. */
const keepsItsUpload = (slot: FileSlot) =>
	slot.state === "sent" && slot.ref !== null && slot.ref.kind !== "inline";

const asReminder = (slot: FileSlot): FileSlot =>
	keepsItsUpload(slot) || slot.state === "reminder"
		? slot
		: { ...slot, state: "reminder", progress: null, ref: null, error: null };

function letFilesGo(state: FormSessionState, tx: Tx): FormSessionState {
	for (const id of slotHolders(state)) tx.released.add(id);
	for (const [id, need] of uploadNeeds(state))
		if (need.slot.state === "sending") tx.aborted.add(id);
	return mapSlots(state, asReminder);
}

/** The form was left: nothing waiting can start any more. */
const detached: InputHandlers<"detached">["detached"] = (state, _input, tx) => {
	const waiting = state.runs.filter(
		(run) => run.origin === "session" && isWaiting(run),
	);
	let next = waiting.reduce(
		(current, run) => notStarted(current, tx, run.id, "formClosed"),
		state,
	);
	next = letFilesGo(next, tx);
	next = withView(withRail(next, { nextFiles: {}, leftOut: {} }), {
		list: null,
		overlay: null,
	});
	next = { ...next, undo: null };
	return next.memory.prefs.auto.length > 0
		? withPrefs(next, tx, { auto: [] })
		: next;
};

const attached: InputHandlers<"attached">["attached"] = (state) => state;

/** Values by field name, fitted to the new fields (spec §5 safe conversions); new fields start at their starting value. */
function rebased(state: FormSessionState, form: FormModel): FieldValues {
	const preset = activePreset(state);
	return Object.fromEntries(
		form.fields.map((field) => {
			const old = state.rail.values[field.name];
			const fitted = old === undefined ? undefined : fitValue(field, old);
			return [field.name, fitted ?? startingValue(field, preset)];
		}),
	);
}

function keepNames<T>(
	record: Readonly<Record<string, T>>,
	names: ReadonlySet<string>,
) {
	return Object.fromEntries(
		Object.entries(record).filter(([name]) => names.has(name)),
	);
}

/** The event or its config changed for real (`contentKey`): values are rebased by name, runs stay. */
const formChanged: InputHandlers<"formChanged">["formChanged"] = (
	state,
	input,
) => {
	const { form } = input;
	const oneFile = new Set(
		form.fields
			.filter((field) => field.kind === "file")
			.map((field) => field.name),
	);
	const keys = new Set(targets(form.fields).map((target) => target.key));
	const target = state.queue.target ?? form.host.fixedTarget;
	const next: FormSessionState = {
		...state,
		form,
		queue: {
			...state.queue,
			target,
			cap: capOf(target, state.queue.tierLimit),
		},
	};
	return withRail(next, {
		values: rebased(state, form),
		nextFiles: keepNames(state.rail.nextFiles, oneFile),
		leftOut: keepNames(state.rail.leftOut, oneFile),
		texts: keepNames(state.rail.texts, keys),
		dateAnchors: keepNames(state.rail.dateAnchors, keys),
		problems: {},
		pressed: false,
	});
};

export const LIFECYCLE_INPUTS: InputHandlers<
	"detached" | "attached" | "formChanged"
> = {
	detached,
	attached,
	formChanged,
};
