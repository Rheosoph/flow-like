/*
 * Presets (spec S1): applying one over another, edits, the menu's summary line, digits, the save
 * dialog's proposed name and ticks, and what a preset saves. No files, no secrets.
 */
import {
	FORM_LIMITS,
	type FieldKind,
	type FieldValue,
	type FieldValues,
	type Preset,
	type ShortWords,
	type StoredInput,
	type WorkbenchField,
} from "../contracts";
import { toStoredInput } from "./stored";
import {
	type IsSecret,
	isFileField,
	presetValue,
	shortText,
	startingValue,
	textOf,
	valueKey,
} from "./values";

const NO_HIDDEN: ReadonlySet<string> = new Set();
const NAME_PROPOSAL_CHARS = 40;
const WEB_ADDRESS = /^([a-z][a-z0-9+.-]*:\/\/|www\.)/i;

export interface PresetChange {
	readonly name: string;
	readonly value: FieldValue;
}

/**
 * `flpPresetChanges`: applying `next` over `prev` (the active preset or null). A field still at the
 * old starting value moves to the new one; an edited field keeps its value unless `next` sets it;
 * files never change.
 */
export function presetChanges(
	fields: readonly WorkbenchField[],
	values: FieldValues,
	prev: Preset | null,
	next: Preset | null,
): readonly PresetChange[] {
	const out: PresetChange[] = [];
	for (const field of fields) {
		if (isFileField(field)) continue;
		const current = values[field.name];
		const untouched =
			valueKey(field, current) === valueKey(field, startingValue(field, prev));
		const target = untouched
			? startingValue(field, next)
			: presetValue(field, next);
		if (
			target !== undefined &&
			valueKey(field, target) !== valueKey(field, current)
		)
			out.push({ name: field.name, value: target });
	}
	return out;
}

/** `flpPresetEdits`: how many inputs the preset sets now differ from it (the edited dot, "Update {name}"). */
export function presetEdits(
	fields: readonly WorkbenchField[],
	values: FieldValues,
	preset: Preset | null,
): number {
	return fields.filter((field) => {
		const set = presetValue(field, preset);
		return (
			set !== undefined &&
			valueKey(field, set) !== valueKey(field, values[field.name])
		);
	}).length;
}

/** Saved inputs of a preset that no longer fit this form ("1 saved input no longer fits this form."). */
export function presetMisfit(
	fields: readonly WorkbenchField[],
	preset: Preset,
): number {
	return Object.keys(preset.sets).filter((name) => {
		const field = fields.find((item) => item.name === name);
		return !field || presetValue(field, preset) === undefined;
	}).length;
}

export interface PresetSummary {
	/** Up to three "{label} {value}" parts, in field order. */
	readonly parts: readonly { readonly label: string; readonly text: string }[];
	/** "+ {more} more". */
	readonly more: number;
}

/** `flpPresetSummary`: the menu row's second line. */
export function presetSummaryParts(
	preset: Preset,
	fields: readonly WorkbenchField[],
	words: ShortWords,
): PresetSummary {
	const parts = fields.flatMap((field) => {
		const value = presetValue(field, preset);
		return value === undefined
			? []
			: [{ label: field.label, text: shortText(field, value, words) }];
	});
	return { parts: parts.slice(0, 3), more: Math.max(0, parts.length - 3) };
}

/** `flpNextDigit`: the lowest digit 1–9 no preset holds, or 0 (no digit). */
export function nextDigit(presets: readonly Pick<Preset, "digit">[]): number {
	const used = new Set(presets.map((preset) => preset.digit));
	for (let digit = 1; digit <= FORM_LIMITS.presetDigits; digit += 1)
		if (!used.has(digit)) return digit;
	return 0;
}

export type PresetNameProposal =
	| { readonly kind: "value"; readonly text: string }
	/** "Preset {n}". */
	| { readonly kind: "numbered"; readonly n: number };

function usableName(text: string): boolean {
	return (
		text !== "" &&
		text.length <= NAME_PROPOSAL_CHARS &&
		!text.includes("\n") &&
		!WEB_ADDRESS.test(text)
	);
}

/**
 * `flpPresetName`: the first ticked one-line text value of 40 characters or fewer that is not a
 * web address and not secret ("Nordwind Logistik GmbH", "K-204418"), else "Preset {n}".
 */
export function presetName(
	fields: readonly WorkbenchField[],
	values: FieldValues,
	ticked: readonly string[],
	existing: readonly Preset[],
	isSecret: IsSecret,
): PresetNameProposal {
	const named = fields.find(
		(field) =>
			ticked.includes(field.name) &&
			field.kind === "text" &&
			!isSecret(field, values[field.name]) &&
			usableName(textOf(values[field.name]).trim()),
	);
	if (!named) return { kind: "numbered", n: existing.length + 1 };
	return { kind: "value", text: textOf(values[named.name]).trim() };
}

function tickable(field: WorkbenchField, perRun: readonly string[]): boolean {
	return (
		!isFileField(field) &&
		field.kind !== "unsupported" &&
		!perRun.includes(field.name)
	);
}

/**
 * `flpPresetTicks`: what the save dialog ticks at first: not files, not per run, not secret, and
 * set by the active preset or different from the default.
 */
export function presetTicks(
	fields: readonly WorkbenchField[],
	values: FieldValues,
	perRun: readonly string[],
	active: Preset | null,
	isSecret: IsSecret,
): readonly string[] {
	return fields
		.filter(
			(field) =>
				tickable(field, perRun) && !isSecret(field, values[field.name]),
		)
		.filter(
			(field) =>
				(active !== null && Object.hasOwn(active.sets, field.name)) ||
				valueKey(field, values[field.name]) !==
					valueKey(field, field.defaultValue),
		)
		.map((field) => field.name);
}

export interface PresetSets {
	readonly sets: Readonly<Record<string, StoredInput>>;
	readonly kinds: Readonly<Record<string, FieldKind>>;
}

/** What a preset saves for the ticked fields: never files, never secrets. */
export function toPresetSets(
	fields: readonly WorkbenchField[],
	values: FieldValues,
	ticked: readonly string[],
	isSecret: IsSecret,
): PresetSets {
	const saved = fields.filter(
		(field) =>
			ticked.includes(field.name) &&
			tickable(field, []) &&
			!isSecret(field, values[field.name]),
	);
	return {
		sets: Object.fromEntries(
			saved.map((field) => [
				field.name,
				toStoredInput(field, values[field.name], NO_HIDDEN),
			]),
		),
		kinds: Object.fromEntries(saved.map((field) => [field.name, field.kind])),
	};
}
