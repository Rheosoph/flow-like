/*
 * Per run (spec M1): which fields go back after a run, the one-time offer, the "Per run" popover
 * rows, the run's pairing and the start message. Next files move in here too (M3).
 */
import {
	type AfterRunBack,
	type AfterRunRow,
	type CopyValue,
	FORM_LIMITS,
	type FieldValue,
	type FieldValues,
	type FileSlot,
	type Preset,
	SECRET_MASK,
	type ShortWords,
	type StartMessage,
	type WorkbenchField,
} from "../contracts";
import {
	type IsSecret,
	emptyValue,
	isEmpty,
	isFileField,
	isHiddenValue,
	shortText,
	startingValue,
	textOf,
	valueKey,
} from "./values";

type Values = Readonly<Record<string, CopyValue>>;
type NextFiles = Readonly<Record<string, readonly FileSlot[]>>;

const isRequiredDate = (field: WorkbenchField) =>
	field.kind === "date" && field.required;

/**
 * `flpAutoPerRun`: what a multi-file pick for `pickedName` makes per run for its series: that
 * field, and every other file field and required date field that is empty at the pick. The
 * reducer uses it only while nothing in the form is per run (else only the picked field).
 */
export function autoPerRun(
	fields: readonly WorkbenchField[],
	pickedName: string,
	values: Values,
): readonly string[] {
	return fields
		.filter(
			(field) =>
				field.name === pickedName ||
				((isFileField(field) || isRequiredDate(field)) &&
					isEmpty(field, values[field.name])),
		)
		.map((field) => field.name);
}

/** "Make files and dates per run": every file field and every required date field. */
export function filesAndDates(
	fields: readonly WorkbenchField[],
): readonly string[] {
	return fields
		.filter((field) => isFileField(field) || isRequiredDate(field))
		.map((field) => field.name);
}

export interface OfferRun {
	readonly values: Values;
	/** Text fields whose entry replaced the value (typed into an empty field or over a full selection). */
	readonly replaced: readonly string[];
}

const OFFER_KINDS: ReadonlySet<string> = new Set(["file", "date", "text"]);

function oneShortLine(value: CopyValue | undefined): boolean {
	const text = textOf(value).trim();
	return text.length <= FORM_LIMITS.offerMaxChars && !text.includes("\n");
}

function replacedIn(field: WorkbenchField, run: OfferRun): boolean {
	if (field.kind !== "text") return true;
	return (
		run.replaced.includes(field.name) && oneShortLine(run.values[field.name])
	);
}

const changedBetween = (field: WorkbenchField, a: OfferRun, b: OfferRun) =>
	valueKey(field, a.values[field.name]) !==
	valueKey(field, b.values[field.name]);

function changedEachTime(
	field: WorkbenchField,
	runs: readonly OfferRun[],
): boolean {
	const [newest, before, oldest] = runs;
	if (
		!changedBetween(field, newest, before) ||
		!changedBetween(field, before, oldest)
	)
		return false;
	return replacedIn(field, newest) && replacedIn(field, before);
}

/**
 * `flpOfferFields`: with 3 or more runs (newest first), the file and date fields and the short
 * one-line text fields whose value was replaced in each of the last two runs; a file among them
 * brings the rest of `autoPerRun` along. Never a per-run or secret field.
 */
export function offerNames(
	fields: readonly WorkbenchField[],
	runsNewestFirst: readonly OfferRun[],
	perRun: readonly string[],
	isSecret: IsSecret,
): readonly string[] {
	if (runsNewestFirst.length < FORM_LIMITS.offerFromRuns) return [];
	const open = fields.filter(
		(field) =>
			OFFER_KINDS.has(field.kind) &&
			!perRun.includes(field.name) &&
			!isSecret(field),
	);
	const base = open.filter((field) => changedEachTime(field, runsNewestFirst));
	if (base.length === 0) return [];
	const file = base.find((field) => field.kind === "file");
	const extra = file
		? autoPerRun(fields, file.name, runsNewestFirst[0].values)
		: [];
	return fields
		.filter(
			(field) =>
				!perRun.includes(field.name) &&
				(base.includes(field) || extra.includes(field.name)),
		)
		.map((field) => field.name);
}

function backOf(
	field: WorkbenchField,
	start: FieldValue,
	nextField: string | null,
	words: ShortWords,
): AfterRunBack {
	if (field.name === nextField) return { kind: "nextFile" };
	if (field.kind === "group" || field.kind === "pairs")
		return { kind: "objectDefault" };
	if (field.kind === "bool") return { kind: start === true ? "on" : "off" };
	if (isEmpty(field, start)) return { kind: "empty" };
	return { kind: "value", text: shortText(field, start, words) };
}

/**
 * `flpAfterRunRows`: the "Per run" popover, one row per field (objects as one row), checked state
 * and what the field goes back to under the active preset ("Back to 20", "Next file", "Empty").
 */
export function afterRunRows(
	fields: readonly WorkbenchField[],
	perRun: readonly string[],
	nextField: string | null,
	preset: Preset | null,
	words: ShortWords,
): readonly AfterRunRow[] {
	return fields.map((field) => ({
		name: field.name,
		label: field.label,
		on: perRun.includes(field.name),
		back: backOf(field, startingValue(field, preset), nextField, words),
	}));
}

export interface PerRunApplied {
	readonly values: FieldValues;
	readonly nextFiles: NextFiles;
	/** The one-file field whose next file moved in, if any. */
	readonly nextFile: string | null;
	/** The run took the last file of a series ("That was the last file."). */
	readonly lastFile: boolean;
}

/**
 * After a run took its copy (M1 step 4): every per-run field goes back to its starting value, and
 * a one-file field with next files takes the next one. A field whose next-files entry exists but
 * is empty holds the series' last file: this run took it, so the entry goes and `lastFile` is set.
 * ⇧⌘↵ (leave as is) skips this entirely.
 */
export function applyPerRun(
	fields: readonly WorkbenchField[],
	values: FieldValues,
	nextFiles: NextFiles,
	perRun: readonly string[],
	baseline: FieldValues,
): PerRunApplied {
	const out: Record<string, FieldValue> = { ...values };
	const queue: Record<string, readonly FileSlot[]> = { ...nextFiles };
	const ended = new Set<string>();
	let nextFile: string | null = null;
	for (const field of fields) {
		const waiting = field.kind === "file" ? queue[field.name] : undefined;
		if (waiting && waiting.length > 0) {
			out[field.name] = waiting[0];
			queue[field.name] = waiting.slice(1);
			nextFile = nextFile ?? field.name;
			continue;
		}
		if (waiting) ended.add(field.name);
		if (perRun.includes(field.name))
			out[field.name] = baseline[field.name] ?? emptyValue(field);
	}
	const left = Object.entries(queue).filter(([name]) => !ended.has(name));
	return {
		values: out,
		nextFiles: Object.fromEntries(left),
		nextFile,
		lastFile: ended.size > 0,
	};
}

function clip(text: string): string {
	const max = FORM_LIMITS.pairingMaxChars;
	return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

/**
 * `flpPairing`: what a run took in its per-run fields, in field order, empty ones left out,
 * secrets as SECRET_MASK: ["invoice-RE-2026-0918.pdf", "18 Sep 2026"].
 */
export function pairing(
	fields: readonly WorkbenchField[],
	values: Values,
	perRun: readonly string[],
	isSecret: IsSecret,
	words: ShortWords,
): readonly string[] {
	const out: string[] = [];
	for (const field of fields) {
		if (!perRun.includes(field.name)) continue;
		const value = values[field.name];
		const hidden = isHiddenValue(value);
		if (!hidden && isEmpty(field, value)) continue;
		out.push(
			hidden || isSecret(field, value)
				? SECRET_MASK
				: clip(shortText(field, value, words)),
		);
	}
	return out;
}

/**
 * `flpStartMessage`: the dock message when a run starts from the rail, or null when there is
 * nothing to say (it started, nothing is per run, the inputs were not left as they are).
 */
export function startMessage(input: StartMessage): StartMessage | null {
	if (input.state === "started" && !input.leftAsIs && input.pairs.length === 0)
		return null;
	return input;
}
