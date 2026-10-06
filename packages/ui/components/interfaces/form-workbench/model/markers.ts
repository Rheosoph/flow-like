/*
 * What a field's label line and edge show (spec 2.0, M1): the dot, the 2 px edge against the
 * compared run, the "Per run" / "Next file" marker, "Optional", hover "Reset" / "Clear", the file
 * count. Rail-against-run comparisons leave per-run fields out.
 */
import type {
	FieldMarkers,
	FieldValue,
	FormSessionState,
	Preset,
	WorkbenchField,
} from "../contracts";
import { pinOptional } from "./fields";
import {
	hasStartingValue,
	presetValue,
	slotsOf,
	splitKey,
	startingValue,
	valueAt,
	valueKey,
} from "./values";

type MarkerState = Pick<FormSessionState, "form" | "rail" | "runs" | "memory">;

/** Fields per run now: the person's own setting and the current series' (`prefs.auto`). */
export function perRunNamesOf(state: Pick<FormSessionState, "memory">) {
	const { perRun, auto } = state.memory.prefs;
	return [...new Set([...perRun, ...auto])];
}

export function activePresetOf(
	state: Pick<FormSessionState, "rail" | "memory">,
) {
	const id = state.rail.activePresetId;
	return state.memory.presets.find((preset) => preset.id === id) ?? null;
}

/** The run the rail is compared with (started in this session, or picked); null for context only. */
export function comparedRunOf(state: Pick<FormSessionState, "rail" | "runs">) {
	const id = state.rail.comparedRunId;
	return id === null ? null : (state.runs.find((run) => run.id === id) ?? null);
}

/** The top-level field a field or property belongs to. */
function ownerOf(fields: readonly WorkbenchField[], field: WorkbenchField) {
	const { name, property } = splitKey(field.key);
	if (property === null) return field;
	return fields.find((item) => item.name === name) ?? field;
}

function startAt(
	owner: WorkbenchField,
	field: WorkbenchField,
	preset: Preset | null,
) {
	const start = startingValue(owner, preset);
	if (owner === field) return start;
	return valueAt({ [owner.name]: start }, field.key) ?? field.defaultValue;
}

function differsFromRun(
	state: MarkerState,
	field: WorkbenchField,
	value: FieldValue | undefined,
) {
	const run = comparedRunOf(state);
	if (!run) return false;
	return (
		valueKey(field, value) !==
		valueKey(field, valueAt(run.copy.values, field.key))
	);
}

const perRunMarker = (
	state: MarkerState,
	field: WorkbenchField,
	perRun: boolean,
): FieldMarkers["perRun"] => {
	if ((state.rail.nextFiles[field.name]?.length ?? 0) > 0) return "nextFile";
	return perRun ? "perRun" : null;
};

/** A property has its own starting value when it has a default or the preset sets its object. */
function ownStart(
	owner: WorkbenchField,
	field: WorkbenchField,
	preset: Preset | null,
) {
	if (owner === field) return hasStartingValue(field, preset);
	return field.hasDefault || presetValue(owner, preset) !== undefined;
}

const resetMarker = (
	differs: boolean,
	hasStart: boolean,
): FieldMarkers["reset"] => {
	if (!differs) return null;
	return hasStart ? "reset" : "clear";
};

function fileCount(field: WorkbenchField, value: FieldValue | undefined) {
	const count = field.kind === "files" ? slotsOf(value).length : 0;
	return count > 0 ? count : null;
}

/**
 * The markers of one field or object property. `changed` (the dot) needs a starting value of its
 * own and never shows on a per-run field; `reset` shows whenever the value differs from its
 * starting value ("Clear" when there is none), typed required fields included.
 */
export function fieldMarkers(
	state: MarkerState,
	field: WorkbenchField,
): FieldMarkers {
	const owner = ownerOf(state.form.fields, field);
	const preset = activePresetOf(state);
	const perRun = perRunNamesOf(state).includes(owner.name);
	const value = valueAt(state.rail.values, field.key);
	const differs =
		valueKey(field, value) !== valueKey(field, startAt(owner, field, preset));
	const hasStart = ownStart(owner, field, preset);
	return {
		changed: hasStart && !perRun && differs,
		differs: !perRun && differsFromRun(state, field, value),
		perRun: owner === field ? perRunMarker(state, field, perRun) : null,
		optional: pinOptional(field) && field.kind !== "bool",
		reset: resetMarker(differs, hasStart),
		count: fileCount(field, value),
	};
}
