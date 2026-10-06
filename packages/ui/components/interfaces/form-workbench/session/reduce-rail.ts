/*
 * Rail edits (SURFACE §6, spec M2, M4): an edit clears its field's message and the dock's Undo; the
 * first change of a text field's entry says how it began (the offer's `replaced`); typed dates commit
 * with `readDate` against the field's anchor; validation runs on blur, on ↵ and on press; "Reset"
 * and "Reset to defaults" put starting values back with Undo.
 */
import type {
	FieldKey,
	FieldValue,
	FieldValues,
	FormSessionState,
} from "../contracts";
import { dateAnchorOf, readDate } from "../model/dates";
import type { FieldTarget } from "../model/fields";
import { targets } from "../model/fields";
import { problemOf } from "../model/validate";
import {
	baselineOf,
	slotsOf,
	startingValue,
	valueAt,
	withValue,
} from "../model/values";
import { removeSlotFrom } from "./reduce-files";
import {
	type CommandHandlers,
	type Tx,
	clearEdits,
	fieldFocus,
	withFocus,
	withProblem,
	withRail,
	withUndoMessage,
	withoutProblem,
	withoutText,
} from "./reduce-tx";
import { activePreset, blockedOf, fieldByName, targetOf } from "./state";

export interface Committed {
	readonly state: FormSessionState;
	/** The typed text could not be read: the field shows its message and keeps the text. */
	readonly unread: boolean;
	/** Days before the anchor of a day alone read far back (↵ then only commits, spec M4). */
	readonly past: number;
}

function commitDate(
	state: FormSessionState,
	target: FieldTarget,
	text: string,
	tx: Tx,
): Committed {
	const { today } = tx.clock;
	const anchor = dateAnchorOf(state, target.key, today);
	const reading = readDate(text, anchor, today, state.form.viewer.dateLocale);
	if (!reading)
		return {
			state: withProblem(state, target.key, { code: "date" }),
			unread: true,
			past: 0,
		};
	const { rail } = state;
	const anchors = reading.iso
		? { ...rail.dateAnchors, [target.key]: reading.iso }
		: rail.dateAnchors;
	const next = withRail(state, {
		values: withValue(rail.values, target.key, reading.iso),
		dateAnchors: anchors,
	});
	return {
		state: withoutProblem(withoutText(next, target.key), target.key),
		unread: false,
		past: reading.past,
	};
}

/** Commits a field's typed text: a date through `readDate`, a number as typed. */
export function commitTyped(
	state: FormSessionState,
	key: FieldKey,
	tx: Tx,
): Committed {
	const text = state.rail.texts[key];
	const target = targetOf(state, key);
	if (text === undefined) return { state, unread: false, past: 0 };
	if (target?.field.kind === "date") return commitDate(state, target, text, tx);
	if (target?.field.kind !== "number")
		return { state: withoutText(state, key), unread: false, past: 0 };
	const next = withRail(state, {
		values: withValue(state.rail.values, key, text),
	});
	return { state: withoutText(next, key), unread: false, past: 0 };
}

/** The field's message as validation finds it now (or none). */
export function validateKey(
	state: FormSessionState,
	key: FieldKey,
): FormSessionState {
	const target = targetOf(state, key);
	if (!target) return state;
	const problem = problemOf(
		target.field,
		valueAt(state.rail.values, key),
		state.rail.texts[key],
		state.form,
	);
	return withProblem(state, key, problem);
}

const ENTRY_BEGAN = { replace: "replaced", inPlace: "inPlace" } as const;

const setValue: CommandHandlers<"setValue">["setValue"] = (state, command) => {
	const target = targetOf(state, command.key);
	if (!target) return state;
	const cleared = clearEdits(state);
	const began = cleared.rail.entryBegan;
	const first =
		command.how !== undefined &&
		target.field.kind === "text" &&
		began[command.key] === undefined;
	const next = withRail(cleared, {
		values: withValue(cleared.rail.values, command.key, command.value),
		entryBegan: first
			? { ...began, [command.key]: ENTRY_BEGAN[command.how ?? "inPlace"] }
			: began,
	});
	return withoutProblem(withoutText(next, command.key), command.key);
};

const setText: CommandHandlers<"setText">["setText"] = (state, command) => {
	if (!targetOf(state, command.key)) return state;
	const next = clearEdits(state);
	return withoutProblem(
		withRail(next, {
			texts: { ...next.rail.texts, [command.key]: command.text },
		}),
		command.key,
	);
};

const commitText: CommandHandlers<"commitText">["commitText"] = (
	state,
	command,
	tx,
) => commitTyped(state, command.key, tx).state;

/** The blur comes from the session moving the cursor on (after a run, a pick, a press), not from the person. */
function movedBySession(state: FormSessionState, key: FieldKey) {
	const target = state.view.focus?.target;
	return (
		target !== undefined && !(target.kind === "field" && target.key === key)
	);
}

/**
 * Leaving a field commits its typed text and validates it. A move the form makes itself does not count as
 * leaving: a per-run date that just went back to empty must not turn red after its run started.
 */
const blurField: CommandHandlers<"blurField">["blurField"] = (
	state,
	command,
	tx,
) => {
	const committed = commitTyped(state, command.key, tx);
	if (committed.unread || movedBySession(state, command.key))
		return committed.state;
	return validateKey(committed.state, command.key);
};

/** What a field or property goes back to: its starting value under the active preset. */
export function startAt(
	state: FormSessionState,
	target: FieldTarget,
): FieldValue {
	const preset = activePreset(state);
	if (target.group === null) return startingValue(target.field, preset);
	const owner = fieldByName(state, target.group);
	const start = owner ? startingValue(owner, preset) : undefined;
	const inside =
		start === undefined
			? undefined
			: valueAt({ [target.group]: start }, target.key);
	return inside ?? target.field.defaultValue;
}

/** ⇧⌘⌫ and hover "Reset": "{label} reset." with Undo; a file field takes its next file (M3). */
const resetField: CommandHandlers<"resetField">["resetField"] = (
	state,
	command,
	tx,
) => {
	const target = targetOf(state, command.key);
	if (!target) return state;
	const current = slotsOf(state.rail.values[target.field.name])[0];
	if (target.field.kind === "file" && current)
		return removeSlotFrom(state, target.field, current.id, tx);
	const values = withValue(
		state.rail.values,
		command.key,
		startAt(state, target),
	);
	const reset = withRail(clearEdits(state), { values });
	const next = withUndoMessage(
		withoutProblem(withoutText(reset, command.key), command.key),
		tx,
		{ kind: "fieldReset", label: target.field.label },
		{ kind: "fieldReset", before: state },
	);
	return withFocus(next, tx, fieldFocus(command.key));
};

/**
 * "Reset to defaults": every field back to its starting value, except a one-file field whose
 * series is running (its current and next files stay, M1). Focus goes to the first field.
 */
const resetAll: CommandHandlers<"resetAll">["resetAll"] = (
	state,
	_command,
	tx,
) => {
	const { fields } = state.form;
	if (fields.length === 0) return state;
	const base = baselineOf(fields, activePreset(state));
	const inSeries = (name: string) => state.rail.nextFiles[name] !== undefined;
	const values: FieldValues = Object.fromEntries(
		fields.map((field) => [
			field.name,
			inSeries(field.name) ? state.rail.values[field.name] : base[field.name],
		]),
	);
	const reset = withRail(clearEdits(state), {
		values,
		texts: {},
		problems: {},
		pressed: false,
	});
	const next = withUndoMessage(
		reset,
		tx,
		{ kind: "resetTo", presetName: null },
		{ kind: "resetAll", before: state },
	);
	const first = targets(fields, blockedOf(state))[0];
	return first ? withFocus(next, tx, fieldFocus(first.key)) : next;
};

const setFilter: CommandHandlers<"setFilter">["setFilter"] = (state, command) =>
	withRail(state, { filter: { ...state.rail.filter, ...command.filter } });

const setRailTab: CommandHandlers<"setRailTab">["setRailTab"] = (
	state,
	command,
) =>
	state.rail.tab === command.tab
		? state
		: withRail(state, { tab: command.tab });

export const RAIL_COMMANDS: CommandHandlers<
	| "setValue"
	| "setText"
	| "commitText"
	| "blurField"
	| "resetField"
	| "resetAll"
	| "setFilter"
	| "setRailTab"
> = {
	setValue,
	setText,
	commitText,
	blurField,
	resetField,
	resetAll,
	setFilter,
	setRailTab,
};
