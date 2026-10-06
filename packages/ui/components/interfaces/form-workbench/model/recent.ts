import {
	FIELD_KEY_SEPARATOR,
	FORM_LIMITS,
	type FieldRecall,
	type FormSessionState,
	type RecentSourceEntry,
	type RecentValue,
	type RunEntry,
	type WorkbenchField,
} from "../contracts";
import { foldText, hashValue, isSecretField, looksSecret } from "./secrets";

export interface RecentOptions {
	/** Values already in the field (list chips). */
	readonly exclude?: readonly string[];
	/** Typed text: a value matches when it or one of its words starts with it. */
	readonly query?: string;
	/** `hashValue`s of values deleted from the list. */
	readonly forgotten?: readonly string[];
	readonly max?: number;
}

/** One-line text fields, text list entries and text properties of objects (spec M6); never secrets. */
function takesRecent(field: WorkbenchField) {
	const textList = field.kind === "chips" && field.itemKind === "text";
	return (field.kind === "text" || textList) && !isSecretField(field, []);
}

function recordOf(value: unknown) {
	if (typeof value !== "object" || value === null || Array.isArray(value))
		return null;
	return value as Readonly<Record<string, unknown>>;
}

/** The field's value in a run: an object property sits inside its object's value. */
function valueIn(entry: RecentSourceEntry, field: WorkbenchField) {
	const cut = field.key.indexOf(FIELD_KEY_SEPARATOR);
	if (cut < 0) return entry.values[field.name];
	return recordOf(entry.values[field.key.slice(0, cut)])?.[field.name];
}

function textOf(value: unknown) {
	if (typeof value === "string") return value.trim();
	if (typeof value === "number" && Number.isFinite(value)) return String(value);
	return "";
}

function entryTexts(entry: RecentSourceEntry, field: WorkbenchField) {
	const raw = valueIn(entry, field);
	const chips = Array.isArray(raw) ? (raw as readonly unknown[]) : [];
	const list = field.kind === "chips" ? chips : [raw];
	return list.map(textOf).filter(Boolean);
}

interface RecentFilter {
	readonly fallback: string | null;
	readonly skip: ReadonlySet<string>;
	readonly gone: ReadonlySet<string>;
	readonly query: string;
}

function filterOf(field: WorkbenchField, options: RecentOptions) {
	const fallback =
		field.kind === "text" && typeof field.defaultValue === "string"
			? foldText(field.defaultValue.trim())
			: null;
	const filter: RecentFilter = {
		fallback,
		skip: new Set((options.exclude ?? []).map((text) => foldText(text.trim()))),
		gone: new Set(options.forgotten ?? []),
		query: options.query ? foldText(options.query).trim() : "",
	};
	return filter;
}

/** One line of at most FORM_LIMITS.recentMaxChars that does not look like a key. */
function fitsTheList(value: string) {
	if (value.length > FORM_LIMITS.recentMaxChars || value.includes("\n"))
		return false;
	return !looksSecret(value);
}

/** Not the default, not already in the field, not deleted from the list. */
function isNew(value: string, folded: string, filter: RecentFilter) {
	if (folded === filter.fallback || filter.skip.has(folded)) return false;
	return !filter.gone.has(hashValue(value));
}

/** The value or one of its words starts with the typed text. */
function matchesQuery(folded: string, query: string) {
	return !query || folded.startsWith(query) || folded.includes(` ${query}`);
}

function offered(value: string, folded: string, filter: RecentFilter) {
	return (
		fitsTheList(value) &&
		isNew(value, folded, filter) &&
		matchesQuery(folded, filter.query)
	);
}

/**
 * Recent values of one text or list field (spec M6, `flpRecent`) from runs newest first, the newest
 * FORM_LIMITS.recentRuns of them: without the field's default, empty or multi-line values, values over
 * FORM_LIMITS.recentMaxChars, values already in the field, forgotten ones, hidden marks and anything secret.
 * Each value once, with the time of the newest run that had it.
 */
export function recentValues(
	source: readonly RecentSourceEntry[],
	field: WorkbenchField,
	options: RecentOptions = {},
) {
	const max = options.max ?? FORM_LIMITS.recentShown;
	if (max <= 0 || !takesRecent(field)) return [];
	return collect(source, field, filterOf(field, options), max);
}

/** Newest first, each value once, until `max` values are found. */
function collect(
	source: readonly RecentSourceEntry[],
	field: WorkbenchField,
	filter: RecentFilter,
	max: number,
) {
	const out: RecentValue[] = [];
	const seen = new Set<string>();
	for (const entry of source.slice(0, FORM_LIMITS.recentRuns)) {
		for (const value of entryTexts(entry, field)) {
			const folded = foldText(value);
			if (seen.has(folded) || !offered(value, folded, filter)) continue;
			seen.add(folded);
			out.push({ value, at: entry.at });
			if (out.length >= max) return out;
		}
	}
	return out;
}

/**
 * The inline suggestion for typed text (spec M6, `flpComplete`): the newest recent value that starts with it,
 * ignoring case and accents, and is longer. Word-start matches belong to the list only. `exclude`: values
 * already in a list field.
 */
export function completion(
	source: readonly RecentSourceEntry[],
	field: WorkbenchField,
	typed: string,
	forgotten: readonly string[],
	exclude: readonly string[] = [],
) {
	const prefix = foldText(typed);
	if (!prefix.trim()) return null;
	const longer = (recent: RecentValue) =>
		foldText(recent.value).startsWith(prefix) &&
		recent.value.length > typed.length;
	const options = { forgotten, exclude, max: FORM_LIMITS.recentRuns };
	return recentValues(source, field, options).find(longer)?.value ?? null;
}

const sources = new WeakMap<
	readonly RunEntry[],
	readonly RecentSourceEntry[]
>();

/** Every field shares one source per runs list: the newest runs' copies, session and history alike. */
function sourceOf(runs: readonly RunEntry[]) {
	const known = sources.get(runs);
	if (known) return known;
	const made: readonly RecentSourceEntry[] = runs
		.slice(0, FORM_LIMITS.recentRuns)
		.map((run) => ({ at: run.createdAt, values: run.copy.values }));
	sources.set(runs, made);
	return made;
}

function groupOf(fields: readonly WorkbenchField[], field: WorkbenchField) {
	const holds = (candidate: WorkbenchField) =>
		candidate.props.some((prop) => prop.key === field.key);
	return fields.find(holds) ?? null;
}

/**
 * What a field's recent values read from, or null for a field without them: other kinds, secrets (also a
 * property of a secret object), "Don't save", and a coarse pointer.
 */
export const recallFor = (
	state: FormSessionState,
	field: WorkbenchField,
	finePointer: boolean,
): FieldRecall | null => {
	const { noSave, forgotten } = state.memory.prefs;
	if (!finePointer || !takesRecent(field) || isSecretField(field, noSave))
		return null;
	const group = groupOf(state.form.fields, field);
	if (group && isSecretField(group, noSave)) return null;
	return {
		source: sourceOf(state.runs),
		forgotten: forgotten[field.key] ?? [],
	};
};
