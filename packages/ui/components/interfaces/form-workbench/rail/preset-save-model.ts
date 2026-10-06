/*
 * The Save dialog's rows and checks (spec S1): which inputs are listed, which start ticked, what is
 * disabled, the name it proposes, and why the primary cannot be pressed. Pure.
 */
import {
	type CopyValue,
	FORM_LIMITS,
	type FormSessionState,
	type Preset,
	type PresetDraft,
	type WorkbenchField,
} from "../contracts";
import { activePresetOf, perRunNamesOf } from "../model/markers";
import {
	type PresetNameProposal,
	presetName,
	presetTicks,
} from "../model/presets";
import { foldText } from "../model/secrets";
import { type IsSecret, isFileField, valueKey } from "../model/values";
import { railValuesOf, secretCheck } from "./rail-model";

export type SaveMode = "save" | "update";

/** Where the dialog reads values from: the rail, or the copy of the run whose menu opened it. */
export interface SaveSource {
	readonly values: Readonly<Record<string, CopyValue>>;
	readonly perRun: readonly string[];
	readonly active: Preset | null;
}

export function saveSourceOf(
	state: FormSessionState,
	fromRunId: string | null,
): SaveSource {
	const run = fromRunId
		? state.runs.find((item) => item.id === fromRunId)
		: undefined;
	if (!run)
		return {
			values: state.rail.values,
			perRun: perRunNamesOf(state),
			active: activePresetOf(state),
		};
	return {
		values: run.copy.values,
		perRun: run.copy.perRun,
		active:
			state.memory.presets.find((item) => item.name === run.copy.presetName) ??
			null,
	};
}

/** Why an input cannot be saved: files are never saved, secrets stay off the device. */
export type SaveRowKind = "free" | "files" | "secret";

export interface SaveRow {
	readonly field: WorkbenchField;
	readonly value: CopyValue | undefined;
	readonly kind: SaveRowKind;
	readonly perRun: boolean;
	/** Holds its default (an empty file field included): listed under "At their defaults". */
	readonly atDefault: boolean;
}

function kindOf(
	field: WorkbenchField,
	value: CopyValue | undefined,
	isSecret: IsSecret,
): SaveRowKind {
	if (isFileField(field)) return "files";
	return isSecret(field, value) ? "secret" : "free";
}

/** Every input the preset could set, in form order; inputs without a control are left out. */
export function saveRowsOf(
	fields: readonly WorkbenchField[],
	source: SaveSource,
	isSecret: IsSecret,
): readonly SaveRow[] {
	return fields
		.filter((field) => field.kind !== "unsupported")
		.map((field) => {
			const value = source.values[field.name];
			return {
				field,
				value,
				kind: kindOf(field, value, isSecret),
				perRun: source.perRun.includes(field.name),
				atDefault:
					valueKey(field, value) === valueKey(field, field.defaultValue),
			};
		});
}

export const isTickable = (row: SaveRow) => row.kind === "free";

/** `flpPresetTicks`: not files, not per run, not secret, and set by the active preset or different from the default. */
export function initialTicks(
	fields: readonly WorkbenchField[],
	source: SaveSource,
	isSecret: IsSecret,
) {
	return presetTicks(
		fields,
		railValuesOf(source.values),
		source.perRun,
		source.active,
		isSecret,
	);
}

export interface SaveStart {
	readonly mode: SaveMode;
	readonly updating: Preset | null;
	readonly source: SaveSource;
	readonly ticks: readonly string[];
	readonly openDefault: boolean;
	/** The proposed name; "Preset {n}" is worded by the view. */
	readonly proposal: PresetNameProposal;
}

/** What the dialog opens with. */
export function saveStartOf(
	state: FormSessionState,
	mode: SaveMode,
	fromRunId: string | null,
): SaveStart {
	const source = saveSourceOf(state, fromRunId);
	const isSecret = secretCheck(state.memory.prefs.noSave);
	const ticks = initialTicks(state.form.fields, source, isSecret);
	const updating = mode === "update" ? source.active : null;
	const taken = state.memory.presets.some((preset) => preset.openDefault);
	return {
		mode,
		updating,
		source,
		ticks,
		openDefault: updating ? updating.openDefault : !taken,
		proposal: updating
			? { kind: "value", text: updating.name }
			: presetName(
					state.form.fields,
					railValuesOf(source.values),
					ticks,
					state.memory.presets,
					isSecret,
				),
	};
}

/** The existing preset a typed name would replace (names are unique per form, ignoring case). */
export function clashOf(
	presets: readonly Preset[],
	name: string,
	updatingId: string | null,
) {
	const folded = foldText(name.trim());
	if (folded === "") return null;
	return (
		presets.find(
			(preset) => preset.id !== updatingId && foldText(preset.name) === folded,
		) ?? null
	);
}

export type SaveBlock = "name" | "nothing" | "limit";

/** Why the primary cannot save yet, or null. */
export function blockOf(input: {
	readonly name: string;
	readonly ticked: number;
	readonly presets: number;
	readonly replaceId: string | null;
}): SaveBlock | null {
	if (input.name.trim() === "") return "name";
	if (input.ticked === 0) return "nothing";
	const adding = input.replaceId === null;
	return adding && input.presets >= FORM_LIMITS.presetsPerForm ? "limit" : null;
}

export function draftOf(input: {
	readonly name: string;
	readonly openDefault: boolean;
	readonly rows: readonly SaveRow[];
	readonly ticked: ReadonlySet<string>;
	readonly fromRunId: string | null;
	readonly replaceId: string | null;
}): PresetDraft {
	return {
		name: input.name.trim(),
		openDefault: input.openDefault,
		ticked: input.rows
			.filter((row) => input.ticked.has(row.field.name))
			.map((row) => row.field.name),
		fromRunId: input.fromRunId,
		replaceId: input.replaceId,
	};
}
