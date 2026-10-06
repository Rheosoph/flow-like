/*
 * Presets (spec S1): applying one over another (fields still at the old starting value move, edited
 * ones keep their value unless the preset sets it, files never change), saving the inputs (from the
 * rail or a run's copy) under a name with the lowest free digit, updating the fields a preset sets,
 * deleting, and resetting to the active preset, each with the one-level Undo.
 */
import {
	FORM_LIMITS,
	type FieldKind,
	type FieldValue,
	type FieldValues,
	type FormSessionState,
	type Preset,
	type PresetDraft,
	type StoredInput,
} from "../contracts";
import { targets } from "../model/fields";
import {
	nextDigit,
	presetChanges,
	presetMisfit,
	toPresetSets,
} from "../model/presets";
import { toStoredInput } from "../model/stored";
import {
	emptyValue,
	isEmpty,
	isFileField,
	isHiddenValue,
	presetValue,
	valueAt,
} from "../model/values";
import {
	type CommandHandlers,
	type Tx,
	clearEdits,
	emit,
	fieldFocus,
	nextSeq,
	withFocus,
	withMessage,
	withPresets,
	withRail,
	withUndoMessage,
	withView,
} from "./reduce-tx";
import { activePreset, blockedOf, runById, secretTest } from "./state";

const NO_HIDDEN: ReadonlySet<string> = new Set();

/** The rail with a preset applied over the active one (`flpPresetChanges`). */
export function applyPresetValues(
	state: FormSessionState,
	preset: Preset | null,
): FieldValues {
	const changes = presetChanges(
		state.form.fields,
		state.rail.values,
		activePreset(state),
		preset,
	);
	if (changes.length === 0) return state.rail.values;
	const values: Record<string, FieldValue> = { ...state.rail.values };
	for (const change of changes) values[change.name] = change.value;
	return values;
}

const closePresetOverlays = (state: FormSessionState) => {
	const id = state.view.overlay?.id;
	return id === "presets" || id === "presetSave"
		? withView(state, { overlay: null })
		: state;
};

function replacePreset(state: FormSessionState, preset: Preset) {
	const others = state.memory.presets.filter((item) => item.id !== preset.id);
	return withPresets(
		state,
		[...others, preset].sort((a, b) => a.createdAt - b.createdAt),
	);
}

function savePresetOf(state: FormSessionState, tx: Tx, preset: Preset) {
	emit(tx, { type: "persistPreset", preset });
	return replacePreset(state, preset);
}

/** After applying: the first empty required field, else back to the Presets button. */
function focusAfterApply(state: FormSessionState, tx: Tx): FormSessionState {
	const values = state.rail.values;
	const empty = targets(state.form.fields, blockedOf(state)).find(
		(target) =>
			target.field.required &&
			target.field.kind !== "unsupported" &&
			isEmpty(target.field, valueAt(values, target.key)),
	);
	return withFocus(
		state,
		tx,
		empty ? fieldFocus(empty.key) : { kind: "presetButton" },
	);
}

/** Click, ↵ or the preset's digit: "{name} applied." with Undo (a misfit count when saved inputs no longer fit). */
const applyPreset: CommandHandlers<"applyPreset">["applyPreset"] = (
	state,
	command,
	tx,
) => {
	const preset =
		state.memory.presets.find((item) => item.id === command.presetId) ?? null;
	if (command.presetId !== null && !preset) return state;
	const applied = closePresetOverlays(
		withRail(clearEdits(state), {
			values: applyPresetValues(state, preset),
			activePresetId: preset?.id ?? null,
			problems: {},
		}),
	);
	if (!preset) return applied;
	const used = savePresetOf(applied, tx, {
		...preset,
		lastUsedAt: tx.clock.now,
	});
	const told = withUndoMessage(
		used,
		tx,
		{
			kind: "presetApplied",
			name: preset.name,
			misfit: presetMisfit(state.form.fields, preset),
		},
		{ kind: "preset", before: state },
	);
	return focusAfterApply(told, tx);
};

/** The values a save takes: the rail, or a run's copy without what was kept out of storage. */
function sourceValues(
	state: FormSessionState,
	draft: PresetDraft,
): { readonly values: FieldValues; readonly hidden: readonly string[] } {
	if (draft.fromRunId === null)
		return { values: state.rail.values, hidden: [] };
	const run = runById(state, draft.fromRunId);
	const values: Record<string, FieldValue> = {};
	const hidden: string[] = [];
	for (const field of state.form.fields) {
		const value = run?.copy.values[field.name];
		if (isHiddenValue(value)) hidden.push(field.name);
		values[field.name] =
			value === undefined || isHiddenValue(value) ? emptyValue(field) : value;
	}
	return { values, hidden };
}

const sameName = (a: string, b: string) =>
	a.localeCompare(b, undefined, { sensitivity: "accent" }) === 0;

/** The preset the draft replaces: the one it names, else one with the same name (ignoring case). */
function replacedBy(state: FormSessionState, draft: PresetDraft, name: string) {
	const { presets } = state.memory;
	if (draft.replaceId !== null)
		return presets.find((preset) => preset.id === draft.replaceId) ?? null;
	return presets.find((preset) => sameName(preset.name, name)) ?? null;
}

/** "On open" is one preset per form: the others lose it (and are saved). */
function onlyOpenDefault(
	state: FormSessionState,
	tx: Tx,
	keep: Preset,
): FormSessionState {
	if (!keep.openDefault) return state;
	return state.memory.presets
		.filter((preset) => preset.id !== keep.id && preset.openDefault)
		.reduce(
			(current, preset) =>
				savePresetOf(current, tx, { ...preset, openDefault: false }),
			state,
		);
}

function presetFromDraft(
	state: FormSessionState,
	draft: PresetDraft,
	name: string,
	tx: Tx,
): Preset {
	const source = sourceValues(state, draft);
	const ticked = draft.ticked.filter((field) => !source.hidden.includes(field));
	const { sets, kinds } = toPresetSets(
		state.form.fields,
		source.values,
		ticked,
		secretTest(state),
	);
	const existing = replacedBy(state, draft, name);
	const now = tx.clock.now;
	if (existing)
		return {
			...existing,
			name,
			sets,
			kinds,
			openDefault: draft.openDefault,
			updatedAt: now,
		};
	return {
		id: `preset-${now.toString(36)}-${nextSeq(tx).toString(36)}`,
		name,
		digit: nextDigit(state.memory.presets),
		sets,
		kinds,
		openDefault: draft.openDefault,
		createdAt: now,
		updatedAt: now,
		lastUsedAt: null,
	};
}

/** ⌘S, the menu or a run's menu: "Saved as {name}."; saved from the rail it becomes the active preset. */
const savePreset: CommandHandlers<"savePreset">["savePreset"] = (
	state,
	command,
	tx,
) => {
	const { draft } = command;
	const name = draft.name.trim().slice(0, FORM_LIMITS.presetNameChars);
	if (!name) return state;
	const replacing = replacedBy(state, draft, name) !== null;
	if (!replacing && state.memory.presets.length >= FORM_LIMITS.presetsPerForm)
		return state;
	const preset = presetFromDraft(state, draft, name, tx);
	let next = onlyOpenDefault(savePresetOf(state, tx, preset), tx, preset);
	if (draft.fromRunId === null)
		next = withRail(next, { activePresetId: preset.id });
	return withMessage(closePresetOverlays(next), tx, {
		kind: "presetSaved",
		name,
	});
};

/** "Update {name}": the current values of the fields the preset sets; it adds none. */
function updatedSets(state: FormSessionState, preset: Preset) {
	const isSecret = secretTest(state);
	const sets: Record<string, StoredInput> = { ...preset.sets };
	const kinds: Record<string, FieldKind> = { ...preset.kinds };
	for (const field of state.form.fields) {
		const value = state.rail.values[field.name];
		if (!(field.name in preset.sets) || isFileField(field)) continue;
		if (isSecret(field, value)) continue;
		sets[field.name] = toStoredInput(field, value, NO_HIDDEN);
		kinds[field.name] = field.kind;
	}
	return { sets, kinds };
}

const updatePreset: CommandHandlers<"updatePreset">["updatePreset"] = (
	state,
	command,
	tx,
) => {
	const preset = state.memory.presets.find(
		(item) => item.id === command.presetId,
	);
	if (!preset) return state;
	const updated: Preset = {
		...preset,
		...updatedSets(state, preset),
		updatedAt: tx.clock.now,
	};
	const next = closePresetOverlays(
		savePresetOf(clearEdits(state), tx, updated),
	);
	return withUndoMessage(
		next,
		tx,
		{ kind: "presetUpdated", name: preset.name },
		{ kind: "update", before: state, deletedPreset: preset },
	);
};

/** The × on a preset row: "{name} deleted." with Undo, which brings it back with its digit. */
const deletePreset: CommandHandlers<"deletePreset">["deletePreset"] = (
	state,
	command,
	tx,
) => {
	const preset = state.memory.presets.find(
		(item) => item.id === command.presetId,
	);
	if (!preset) return state;
	emit(tx, { type: "deletePreset", presetId: preset.id });
	const kept = withPresets(
		clearEdits(state),
		state.memory.presets.filter((item) => item.id !== preset.id),
	);
	const active =
		state.rail.activePresetId === preset.id ? null : state.rail.activePresetId;
	return withUndoMessage(
		withRail(kept, { activePresetId: active }),
		tx,
		{ kind: "presetDeleted", name: preset.name },
		{ kind: "deletePreset", before: state, deletedPreset: preset },
	);
};

/** "Reset to {name}": the fields the active preset sets go back to its values. */
const resetToPreset: CommandHandlers<"resetToPreset">["resetToPreset"] = (
	state,
	_command,
	tx,
) => {
	const preset = activePreset(state);
	if (!preset) return state;
	const values: Record<string, FieldValue> = { ...state.rail.values };
	for (const field of state.form.fields) {
		const set = presetValue(field, preset);
		if (set !== undefined) values[field.name] = set;
	}
	const reset = closePresetOverlays(
		withRail(clearEdits(state), { values, problems: {}, texts: {} }),
	);
	return withUndoMessage(
		reset,
		tx,
		{ kind: "resetTo", presetName: preset.name },
		{ kind: "resetTo", before: state },
	);
};

/** Undo of a deleted or updated preset puts the earlier version back and saves it. */
export function restorePreset(
	state: FormSessionState,
	tx: Tx,
	kind: string,
	preset: Preset | null,
): FormSessionState {
	if (!preset || (kind !== "deletePreset" && kind !== "update")) return state;
	return savePresetOf(state, tx, preset);
}

export const PRESET_COMMANDS: CommandHandlers<
	| "applyPreset"
	| "savePreset"
	| "updatePreset"
	| "deletePreset"
	| "resetToPreset"
> = {
	applyPreset,
	savePreset,
	updatePreset,
	deletePreset,
	resetToPreset,
};
