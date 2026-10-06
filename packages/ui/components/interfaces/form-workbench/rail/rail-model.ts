/*
 * What the rail decides before it draws (PLAN §7, spec S1, S5, M1, M5): which parts show, how fields
 * pair into rows, what the filter keeps, how runs group by day and what their rows say. Pure.
 */
import {
	type CopyValue,
	FORM_LIMITS,
	type FieldKey,
	type FieldMarkers,
	type FieldValues,
	type FormSessionState,
	type NotStartedReason,
	type RailFilter,
	type RailTab,
	type RunEntry,
	type ShortWords,
	type StepRef,
	type WorkbenchField,
	type WorkbenchLayout,
	type WorkbenchViewProps,
} from "../contracts";
import { type WhenWords, formatWhen } from "../model/date-text";
import { targets } from "../model/fields";
import { fieldMarkers, perRunNamesOf } from "../model/markers";
import { foldText, hiddenNames } from "../model/secrets";
import {
	type IsSecret,
	changedNames,
	isHiddenValue,
	splitKey,
} from "../model/values";
import { stageRunOf } from "../run/run-view";

/** What every part of the rail renders from. */
export type RailPartProps = Pick<
	WorkbenchViewProps,
	"state" | "actions" | "layout"
>;

// ─── What shows ─────────────────────────────────────────────────────────────

export type DescriptionMode = "hidden" | "full" | "clamped";

export interface RailShape {
	/** Inputs | Runs: from two runs, and only beside a stage. */
	readonly tabs: boolean;
	readonly tab: RailTab;
	/** The filter row sits above the field list of a large form. */
	readonly filter: boolean;
	/** The Presets button (S1 visibility rule). */
	readonly presets: boolean;
	readonly description: DescriptionMode;
	/** The after-run row at the end of a narrow Inputs pane (M1, phone). */
	readonly afterRun: boolean;
}

/** The Runs list earns its place in the rail once there is more than one run to choose from. */
const TABS_FROM_RUNS = 2;

const hasNextFiles = (state: Pick<FormSessionState, "rail">) =>
	Object.values(state.rail.nextFiles).some((files) => files.length > 0);

/** Per-run settings that name a field this form still has. */
export function perRunCount(state: FormSessionState) {
	const names = new Set(state.form.fields.map((field) => field.name));
	return perRunNamesOf(state).filter((name) => names.has(name)).length;
}

/** M1: the setting has been shown on this device, so the after-run line exists from now on. */
export function isIntroduced(state: FormSessionState) {
	return (
		state.memory.prefs.introduced ||
		perRunCount(state) > 0 ||
		hasNextFiles(state) ||
		state.view.question !== null
	);
}

/** S1: a preset on this device, or a large form with a history. */
export function showsPresets(state: FormSessionState) {
	return (
		state.memory.presets.length > 0 ||
		(state.form.fields.length >= FORM_LIMITS.presetsFromFields &&
			state.runs.length >= FORM_LIMITS.presetsFromRuns)
	);
}

function descriptionOf(
	state: FormSessionState,
	layout: WorkbenchLayout,
): DescriptionMode {
	if (!state.form.description) return "hidden";
	if (!layout.split) return "full";
	if (stageRunOf(state) === null) return "hidden";
	return state.runs.length >= TABS_FROM_RUNS ? "clamped" : "full";
}

export function railShapeOf(
	state: FormSessionState,
	layout: WorkbenchLayout,
): RailShape {
	const tabs = layout.split && state.runs.length >= TABS_FROM_RUNS;
	const tab = tabs ? state.rail.tab : "inputs";
	return {
		tabs,
		tab,
		filter:
			tab === "inputs" &&
			state.form.fields.length >= FORM_LIMITS.filterFromFields,
		presets: showsPresets(state),
		description: descriptionOf(state, layout),
		afterRun: !layout.split && isIntroduced(state),
	};
}

// ─── Markers ────────────────────────────────────────────────────────────────

export const NO_MARKERS: FieldMarkers = {
	changed: false,
	differs: false,
	perRun: null,
	optional: false,
	reset: null,
	count: null,
};

/** Every field and object property by key; fields and properties are found only through here. */
export function fieldIndex(fields: readonly WorkbenchField[]) {
	const index = new Map<FieldKey, WorkbenchField>();
	for (const field of fields) index.set(field.key, field);
	for (const target of targets(fields)) index.set(target.key, target.field);
	return index;
}

type MarkerState = Pick<FormSessionState, "form" | "rail" | "runs" | "memory">;

export function markerMap(
	state: MarkerState,
	index: ReadonlyMap<FieldKey, WorkbenchField>,
) {
	const map = new Map<FieldKey, FieldMarkers>();
	for (const [key, field] of index) map.set(key, fieldMarkers(state, field));
	return map;
}

/** "Reset to defaults" shows while any field differs from its starting value, typed required fields included. */
export function anyFieldDiffers(
	fields: readonly WorkbenchField[],
	markers: ReadonlyMap<FieldKey, FieldMarkers>,
) {
	return fields.some(
		(field) => (markers.get(field.key)?.reset ?? null) !== null,
	);
}

// ─── The filter ─────────────────────────────────────────────────────────────

export interface FilterCounts {
	readonly all: number;
	readonly required: number;
	readonly changed: number;
}

export function changedSet(
	state: Pick<FormSessionState, "form" | "rail" | "memory">,
) {
	const preset =
		state.memory.presets.find(
			(item) => item.id === state.rail.activePresetId,
		) ?? null;
	return new Set(
		changedNames(
			state.form.fields,
			state.rail.values,
			preset,
			perRunNamesOf(state),
		),
	);
}

export function filterCounts(
	fields: readonly WorkbenchField[],
	changed: ReadonlySet<string>,
): FilterCounts {
	return {
		all: fields.length,
		required: fields.filter((field) => field.required).length,
		changed: fields.filter((field) => changed.has(field.name)).length,
	};
}

const hasText = (field: WorkbenchField, needle: string) =>
	foldText(field.label).includes(needle) ||
	foldText(field.name).includes(needle);

/** The fields the filter keeps, in form order: the chip, then the typed text against label and name. */
export function fieldsMatching(
	fields: readonly WorkbenchField[],
	filter: RailFilter,
	changed: ReadonlySet<string>,
) {
	const needle = foldText(filter.query).trim();
	return fields.filter((field) => {
		if (filter.chip === "required" && !field.required) return false;
		if (filter.chip === "changed" && !changed.has(field.name)) return false;
		return needle === "" || hasText(field, needle);
	});
}

export const isFiltering = (filter: RailFilter) =>
	filter.chip !== "all" || filter.query.trim() !== "";

/** The name of the top-level field a focus request needs on screen, or null. */
export function focusOwnerName(
	state: Pick<FormSessionState, "view">,
): string | null {
	const target = state.view.focus?.target;
	return target?.kind === "field" ? splitKey(target.key).name : null;
}

// ─── The grid ───────────────────────────────────────────────────────────────

/** 13 px Inter 500 measures about 6.6 px a character; a half cell of the 400 px rail is 174 px. */
const CHAR_WIDTH = 6.6;
const HALF_CELL = 174;
const OPTIONAL_WIDTH = 60;
const DOT_WIDTH = 12;

/**
 * Whether a short field's label line (label, dot, "Optional") fits half a row; a field that does not
 * fit takes the whole row instead of cutting its label. The per-run marker is not counted: it turns
 * into its icon in a narrow cell, and a field must not change rows when it becomes per run.
 */
export function fitsHalfRow(field: WorkbenchField, markers: FieldMarkers) {
	if (!field.short) return false;
	const optional = markers.optional ? OPTIONAL_WIDTH : 0;
	return field.label.length * CHAR_WIDTH + optional + DOT_WIDTH <= HALF_CELL;
}

/** Columns a field takes in the grid: 1 when two fit a row, else 2. A single-column rail gives every field its row. */
export function columnsOf(
	field: WorkbenchField,
	markers: FieldMarkers,
	split: boolean,
): 1 | 2 {
	return split && fitsHalfRow(field, markers) ? 1 : 2;
}

// ─── Presets ────────────────────────────────────────────────────────────────

/**
 * Whether a field, or the value it holds, is kept out of storage: a secret field, "Don't save", a hidden
 * value or key-like text. An object counts when any of its properties does.
 */
export function secretCheck(noSave: readonly string[]): IsSecret {
	return (field, value) =>
		hiddenNames([field], { [field.name]: value ?? null }, noSave).size > 0;
}

/** A copy's values as rail values: a hidden value is nothing the rail could set. */
export function railValuesOf(
	copy: Readonly<Record<string, CopyValue>>,
): FieldValues {
	return Object.fromEntries(
		Object.entries(copy).map(([name, value]) => [
			name,
			isHiddenValue(value) ? null : value,
		]),
	);
}

// ─── Runs ───────────────────────────────────────────────────────────────────

const DAY_MS = 86_400_000;

/** The calendar day of a moment in the viewer's time zone, as a day count. */
export function localDayNumber(at: number) {
	const date = new Date(at);
	return Math.round(
		Date.UTC(date.getFullYear(), date.getMonth(), date.getDate()) / DAY_MS,
	);
}

/** `YYYY-MM-DD` of the viewer's day. */
export function localDay(date: Date = new Date()) {
	const month = String(date.getMonth() + 1).padStart(2, "0");
	const day = String(date.getDate()).padStart(2, "0");
	return `${date.getFullYear()}-${month}-${day}`;
}

export function dayHeading(
	at: number,
	now: number,
	locale: string,
	words: WhenWords,
) {
	if (localDayNumber(at) === localDayNumber(now)) return words.today;
	return formatWhen(at, now, locale, words);
}

const isWaiting = (run: Pick<RunEntry, "status">) =>
	run.status === "queued" || run.status === "sending";

export interface RunGroup {
	readonly day: number;
	/** A moment on that day, for the heading. */
	readonly at: number;
	readonly runs: readonly RunEntry[];
}

/**
 * Runs grouped by day, newest first. Queued and sending runs head "Today" whatever their day (M5);
 * every other run goes under the day it was pressed.
 */
export function groupRunsByDay(
	runs: readonly RunEntry[],
	now: number,
): readonly RunGroup[] {
	const groups: { day: number; at: number; runs: RunEntry[] }[] = [];
	const waiting = runs.filter(isWaiting);
	if (waiting.length > 0)
		groups.push({ day: localDayNumber(now), at: now, runs: waiting });
	for (const run of runs.filter((item) => !isWaiting(item))) {
		const day = localDayNumber(run.createdAt);
		const last = groups.at(-1);
		if (last && last.day === day) last.runs.push(run);
		else groups.push({ day, at: run.createdAt, runs: [run] });
	}
	return groups;
}

/** "1st", "2nd", "3rd", "4th", "11th", "21st": the suffix an English reader expects. */
export function ordinalSuffix(n: number): "st" | "nd" | "rd" | "th" {
	const tens = n % 100;
	if (tens >= 11 && tens <= 13) return "th";
	if (n % 10 === 1) return "st";
	if (n % 10 === 2) return "nd";
	return n % 10 === 3 ? "rd" : "th";
}

export interface ChangeItem {
	readonly label: string;
	readonly from: string;
	readonly to: string;
}

/** What a run's inputs say against the run pressed before it. */
export type RunChanges =
	| { readonly kind: "first" }
	| { readonly kind: "same"; readonly n: number }
	| { readonly kind: "diff"; readonly items: readonly ChangeItem[] };

export interface ChangeContext {
	readonly fields: readonly WorkbenchField[];
	readonly words: ShortWords;
	readonly isSecret: IsSecret;
}

/** "Run OCR: Off → On · Max pages: 20 → 60". */
export const changeText = (items: readonly ChangeItem[]) =>
	items.map((item) => `${item.label}: ${item.from} → ${item.to}`).join(" · ");

/** The row's third line, as a descriptor the view words. */
export type RunPreview =
	| { readonly kind: "text"; readonly text: string }
	| { readonly kind: "failedAt"; readonly step: StepRef }
	| { readonly kind: "stoppedAt"; readonly step: StepRef }
	| { readonly kind: "notStarted"; readonly reason: NotStartedReason }
	| { readonly kind: "unknown" }
	| { readonly kind: "nothing" };

const textPreview = (run: RunEntry): RunPreview | null =>
	run.summary.firstLine ? { kind: "text", text: run.summary.firstLine } : null;

const atStep = (run: RunEntry) => run.failedAt ?? run.summary.stepReached;

function endedPreview(run: RunEntry): RunPreview | null {
	const step = atStep(run);
	if (run.status === "failed")
		return step ? { kind: "failedAt", step } : textPreview(run);
	if (run.status === "stopped")
		return step ? { kind: "stoppedAt", step } : textPreview(run);
	if (run.status === "empty") return textPreview(run) ?? { kind: "nothing" };
	return textPreview(run);
}

export function previewOf(run: RunEntry): RunPreview | null {
	if (run.status === "notStarted") {
		const outcome = run.outcome;
		return outcome?.kind === "notStarted"
			? { kind: "notStarted", reason: outcome.reason }
			: null;
	}
	if (run.status === "unknown") return { kind: "unknown" };
	return endedPreview(run);
}

/** The wait the time column names: the place in line, or that the API had no place for it. */
export type RunWait =
	| { readonly kind: "place"; readonly place: number }
	| { readonly kind: "noPlace" };

export function waitOf(
	run: RunEntry,
	place: number | undefined,
): RunWait | null {
	if (run.status !== "queued") return null;
	if (run.waitingForPlace) return { kind: "noPlace" };
	return place === undefined ? null : { kind: "place", place };
}
