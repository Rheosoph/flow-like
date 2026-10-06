import {
	type FormMemoryRow,
	type FormMemoryTableName,
	type FormMemoryTables,
	dexieFormMemoryTables,
} from "../../../../db/form-memory-db";
import {
	type CreateFormMemoryStore,
	FIELD_KEY_SEPARATOR,
	FORM_LIMITS,
	type FormMemoryKey,
	type FormMemoryStore,
	type FormPersistence,
	type FormPrefs,
	type LoadedMemory,
	MEMORY_DB_NAME,
	type Preset,
	type StoredHiddenMark,
	type StoredInput,
	type StoredRunRecord,
	type StoredRunStatus,
} from "../contracts";
import { isSecretField, looksSecret } from "../model/secrets";

const EMPTY_MEMORY: LoadedMemory = { prefs: null, presets: [], runs: [] };
const HIDDEN: StoredHiddenMark = { $hidden: true };
const PREFS_ROW_ID = "";
const STORED_RUN_STATUSES: readonly StoredRunStatus[] = [
	"queued",
	"sending",
	"running",
	"done",
	"empty",
	"failed",
	"stopped",
	"notStarted",
];

type JsonRecord = Readonly<Record<string, unknown>>;
type StoredInputs = Readonly<Record<string, StoredInput>>;

// ─── Reading rows ───────────────────────────────────────────────────────────

function parseJson(json: string): unknown {
	try {
		return JSON.parse(json);
	} catch {
		return null;
	}
}

function asRecord(value: unknown) {
	if (typeof value !== "object" || value === null || Array.isArray(value))
		return null;
	return value as JsonRecord;
}

const isFiniteNumber = (value: unknown): value is number =>
	typeof value === "number" && Number.isFinite(value);

const isStringList = (value: unknown): value is string[] =>
	Array.isArray(value) && value.every((item) => typeof item === "string");

const isOptionalTime = (value: unknown) =>
	value === null || isFiniteNumber(value);

function hasRunShape(value: JsonRecord) {
	return (
		typeof value.id === "string" &&
		isFiniteNumber(value.n) &&
		isFiniteNumber(value.createdAt) &&
		STORED_RUN_STATUSES.includes(value.status as StoredRunStatus) &&
		asRecord(value.summary) !== null
	);
}

function hasRunInputs(value: JsonRecord) {
	return (
		asRecord(value.inputs) !== null &&
		isStringList(value.perRun) &&
		isStringList(value.replaced)
	);
}

/** A stored run record of this version, or null for a row written by another version or damaged. */
export function runOf(json: string) {
	const value = asRecord(parseJson(json));
	if (!value || value.version !== 2) return null;
	if (!hasRunShape(value) || !hasRunInputs(value)) return null;
	return value as unknown as StoredRunRecord;
}

function hasPresetShape(value: JsonRecord) {
	return (
		typeof value.id === "string" &&
		typeof value.name === "string" &&
		isFiniteNumber(value.digit) &&
		typeof value.openDefault === "boolean" &&
		asRecord(value.sets) !== null &&
		asRecord(value.kinds) !== null
	);
}

function hasPresetTimes(value: JsonRecord) {
	return (
		isFiniteNumber(value.createdAt) &&
		isFiniteNumber(value.updatedAt) &&
		isOptionalTime(value.lastUsedAt)
	);
}

/** A stored preset, or null for a damaged row. */
export function presetOf(json: string) {
	const value = asRecord(parseJson(json));
	if (!value || !hasPresetShape(value) || !hasPresetTimes(value)) return null;
	return value as unknown as Preset;
}

function stringLists(value: unknown) {
	const out: Record<string, readonly string[]> = {};
	for (const [name, list] of Object.entries(asRecord(value) ?? {})) {
		if (isStringList(list)) out[name] = list;
	}
	return out;
}

function times(value: unknown) {
	const out: Record<string, number> = {};
	for (const [name, at] of Object.entries(asRecord(value) ?? {})) {
		if (isFiniteNumber(at)) out[name] = at;
	}
	return out;
}

const listOrEmpty = (value: unknown): readonly string[] =>
	isStringList(value) ? value : [];

/** Prefs of this version with every part checked; a damaged part reads as its empty value. */
export function prefsOf(json: string) {
	const value = asRecord(parseJson(json));
	if (!value || value.version !== 2) return null;
	const prefs: FormPrefs = {
		version: 2,
		perRun: listOrEmpty(value.perRun),
		auto: listOrEmpty(value.auto),
		introduced: value.introduced === true,
		offerAnswered: value.offerAnswered === true,
		noSave: listOrEmpty(value.noSave),
		forgotten: stringLists(value.forgotten),
		nextRunNumber: isFiniteNumber(value.nextRunNumber)
			? value.nextRunNumber
			: 1,
		fieldSeenAt: times(value.fieldSeenAt),
	};
	return prefs;
}

// ─── Rules applied on write ─────────────────────────────────────────────────

/**
 * Drops settings of field names unseen for FORM_LIMITS.prefsPruneMs (spec §5: names the form no longer has
 * are ignored, then pruned after 90 days). Names without a `fieldSeenAt` stamp are kept.
 */
export function prunePrefs(prefs: FormPrefs, now: number) {
	const cutoff = now - FORM_LIMITS.prefsPruneMs;
	const stale = new Set(
		Object.entries(prefs.fieldSeenAt)
			.filter(([, at]) => at < cutoff)
			.map(([name]) => name),
	);
	if (stale.size === 0) return prefs;
	const keep = (name: string) => !stale.has(name);
	const kept = <T>(record: Readonly<Record<string, T>>) =>
		Object.fromEntries(Object.entries(record).filter(([name]) => keep(name)));
	const pruned: FormPrefs = {
		...prefs,
		perRun: prefs.perRun.filter(keep),
		auto: prefs.auto.filter(keep),
		noSave: prefs.noSave.filter(keep),
		forgotten: kept(prefs.forgotten),
		fieldSeenAt: kept(prefs.fieldSeenAt),
	};
	return pruned;
}

const isHiddenMark = (value: unknown) => asRecord(value)?.$hidden === true;

const isFileMark = (value: unknown) => asRecord(value)?.$file !== undefined;

/** Every text inside a stored value, file marks and hidden marks left out. */
const textsWithin = (value: unknown): string[] => {
	if (typeof value === "string") return [value];
	if (Array.isArray(value)) return value.flatMap(textsWithin);
	const record = asRecord(value);
	if (!record || isHiddenMark(record) || isFileMark(record)) return [];
	return Object.values(record).flatMap(textsWithin);
};

const isSecretName = (name: string) =>
	isSecretField({ key: name, name, label: name, sensitive: false }, []);

/**
 * The store's own safety net under the reducer's rules (spec M6): an input named like a secret, or holding a
 * text that looks like a key, is written as `{ $hidden: true }`. The reducer already hides by field
 * (`sensitive`, "Don't save", labels, properties); this catches what reaches storage anyway.
 */
export function withoutSecrets(inputs: StoredInputs) {
	const out: Record<string, StoredInput> = {};
	for (const [name, value] of Object.entries(inputs)) {
		const secret =
			!isHiddenMark(value) &&
			(isSecretName(name) || textsWithin(value).some(looksSecret));
		out[name] = secret ? HIDDEN : value;
	}
	return out;
}

const encoder = new TextEncoder();
const bytesOf = (text: string) => encoder.encode(text).length;

const fits = (preset: Preset) =>
	bytesOf(JSON.stringify(preset)) <= FORM_LIMITS.presetBytes;

function withoutNames(preset: Preset, dropped: ReadonlySet<string>) {
	const keep = ([name]: [string, unknown]) => !dropped.has(name);
	const kept: Preset = {
		...preset,
		sets: Object.fromEntries(Object.entries(preset.sets).filter(keep)),
		kinds: Object.fromEntries(Object.entries(preset.kinds).filter(keep)),
	};
	return kept;
}

/**
 * A preset within FORM_LIMITS.presetBytes: when its JSON is larger, the largest values it sets are left out
 * until it fits (name, digit and flags stay).
 */
export function fitPreset(preset: Preset) {
	if (fits(preset)) return preset;
	const size = (name: string) => bytesOf(JSON.stringify(preset.sets[name]));
	const largestFirst = Object.keys(preset.sets).sort(
		(a, b) => size(b) - size(a),
	);
	const dropped = new Set<string>();
	for (const name of largestFirst) {
		dropped.add(name);
		const fitted = withoutNames(preset, dropped);
		if (fits(fitted)) return fitted;
	}
	return withoutNames(preset, dropped);
}

function hideProperty(
	inputs: StoredInputs,
	groupName: string,
	property: string,
) {
	const group = asRecord(inputs[groupName]);
	if (!group || isHiddenMark(group) || !Object.hasOwn(group, property))
		return null;
	if (isHiddenMark(group[property])) return null;
	const hidden: StoredInputs = {
		...inputs,
		[groupName]: { ...group, [property]: HIDDEN } as StoredInput,
	};
	return hidden;
}

/** `name` is a field name, or `<object>${FIELD_KEY_SEPARATOR}<property>` for one property of an object input. */
function hideInput(inputs: StoredInputs, name: string) {
	const cut = name.indexOf(FIELD_KEY_SEPARATOR);
	if (cut >= 0)
		return hideProperty(
			inputs,
			name.slice(0, cut),
			name.slice(cut + FIELD_KEY_SEPARATOR.length),
		);
	if (!Object.hasOwn(inputs, name) || isHiddenMark(inputs[name])) return null;
	const hidden: StoredInputs = { ...inputs, [name]: HIDDEN };
	return hidden;
}

function newestFirst(a: StoredRunRecord, b: StoredRunRecord) {
	return b.createdAt - a.createdAt || b.n - a.n;
}

function oldestFirst(a: Preset, b: Preset) {
	return a.createdAt - b.createdAt || a.digit - b.digit;
}

const keyOf = (record: StoredRunRecord): FormMemoryKey => ({
	scope: record.scope,
	appId: record.appId,
	eventId: record.eventId,
});

function parsedRows<T>(
	rows: readonly FormMemoryRow[],
	parse: (json: string) => T | null,
) {
	const out: T[] = [];
	for (const row of rows) {
		const value = parse(row.json);
		if (value !== null) out.push(value);
	}
	return out;
}

// ─── The store ──────────────────────────────────────────────────────────────

async function loadForm(
	tables: FormMemoryTables,
	key: FormMemoryKey,
	now: number,
) {
	const [prefsRows, presetRows, runRows] = await Promise.all([
		tables.read("prefs", key),
		tables.read("presets", key),
		tables.read("runs", key),
	]);
	const prefs = parsedRows(prefsRows, prefsOf)[0] ?? null;
	const memory: LoadedMemory = {
		prefs: prefs ? prunePrefs(prefs, now) : null,
		presets: parsedRows(presetRows, presetOf).sort(oldestFirst),
		runs: parsedRows(runRows, runOf)
			.sort(newestFirst)
			.slice(0, FORM_LIMITS.historyPerForm),
	};
	return memory;
}

/** Writes the record and keeps the form's newest FORM_LIMITS.historyPerForm runs. */
async function putRunRow(tables: FormMemoryTables, record: StoredRunRecord) {
	const key = keyOf(record);
	const stored = { ...record, inputs: withoutSecrets(record.inputs) };
	const row = {
		id: record.id,
		createdAt: record.createdAt,
		json: JSON.stringify(stored),
	};
	await tables.transaction(async () => {
		await tables.write("runs", key, [row]);
		const ids = await tables.runIdsOldestFirst(key);
		const extra = ids.length - FORM_LIMITS.historyPerForm;
		if (extra > 0) await tables.remove("runs", key, ids.slice(0, extra));
	});
}

/** The row with `name` hidden; none when the row has nothing to hide. */
function hiddenRow(row: FormMemoryRow, name: string) {
	const record = runOf(row.json);
	const inputs = record ? hideInput(record.inputs, name) : null;
	if (!record || !inputs) return [];
	return [{ ...row, json: JSON.stringify({ ...record, inputs }) }];
}

/** "Don't save {label}": that input becomes `{ $hidden: true }` in every saved run of the form. */
async function hideFieldRows(
	tables: FormMemoryTables,
	key: FormMemoryKey,
	name: string,
) {
	await tables.transaction(async () => {
		const rows = await tables.read("runs", key);
		const changed = rows.flatMap((row) => hiddenRow(row, name));
		if (changed.length > 0) await tables.write("runs", key, changed);
	});
}

/** Last writer wins, except `nextRunNumber`: the larger of stored and written, so two windows never reuse a number. */
async function putPrefsRow(
	tables: FormMemoryTables,
	key: FormMemoryKey,
	prefs: FormPrefs,
	now: number,
) {
	await tables.transaction(async () => {
		const stored = parsedRows(await tables.read("prefs", key), prefsOf)[0];
		const nextRunNumber = Math.max(
			prefs.nextRunNumber,
			stored?.nextRunNumber ?? 0,
		);
		const merged = prunePrefs({ ...prefs, nextRunNumber }, now);
		await tables.write("prefs", key, [
			{ id: PREFS_ROW_ID, createdAt: 0, json: JSON.stringify(merged) },
		]);
	});
}

async function putPresetRow(
	tables: FormMemoryTables,
	key: FormMemoryKey,
	preset: Preset,
) {
	const stored = fitPreset({ ...preset, sets: withoutSecrets(preset.sets) });
	await tables.write("presets", key, [
		{
			id: preset.id,
			createdAt: preset.createdAt,
			json: JSON.stringify(stored),
		},
	]);
}

async function removeRow(
	tables: FormMemoryTables,
	table: FormMemoryTableName,
	key: FormMemoryKey,
	id: string,
) {
	await tables.remove(table, key, [id]);
}

/**
 * The memory store over any row tables: Dexie on the device, a Map for the page session and tests. Every method
 * swallows storage errors (private windows, blocked storage) and reports the first one once: the form keeps
 * working from memory.
 */
export function createMemoryStoreOn(
	tables: FormMemoryTables,
	mode: FormPersistence,
	clock: () => number = Date.now,
) {
	let reported = false;
	const safely = async <T>(work: () => Promise<T>, fallback: T) => {
		try {
			return await work();
		} catch (error) {
			if (!reported) {
				reported = true;
				console.warn(
					`Form memory (${mode}) could not use its storage; this form keeps its runs in memory only.`,
					error,
				);
			}
			return fallback;
		}
	};
	const store: FormMemoryStore = {
		mode,
		load: (key) => safely(() => loadForm(tables, key, clock()), EMPTY_MEMORY),
		putRun: (record) => safely(() => putRunRow(tables, record), undefined),
		deleteRun: (key, id) =>
			safely(() => removeRow(tables, "runs", key, id), undefined),
		hideField: (key, name) =>
			safely(() => hideFieldRows(tables, key, name), undefined),
		putPrefs: (key, prefs) =>
			safely(() => putPrefsRow(tables, key, prefs, clock()), undefined),
		putPreset: (key, preset) =>
			safely(() => putPresetRow(tables, key, preset), undefined),
		deletePreset: (key, presetId) =>
			safely(() => removeRow(tables, "presets", key, presetId), undefined),
	};
	return store;
}

// ─── Session memory ─────────────────────────────────────────────────────────

type FormRows = Record<FormMemoryTableName, Map<string, FormMemoryRow>>;

const SESSION_ENTRY = `${MEMORY_DB_NAME}:forms`;
const pageSession = new Map<string, unknown>();

const formId = (key: FormMemoryKey) =>
	JSON.stringify([key.scope, key.appId, key.eventId]);

function formsIn(map: Map<string, unknown>) {
	const known = map.get(SESSION_ENTRY);
	if (known instanceof Map) return known as Map<string, FormRows>;
	const made = new Map<string, FormRows>();
	map.set(SESSION_ENTRY, made);
	return made;
}

function rowsIn(map: Map<string, unknown>, key: FormMemoryKey) {
	const forms = formsIn(map);
	const id = formId(key);
	const known = forms.get(id);
	if (known) return known;
	const made: FormRows = {
		runs: new Map(),
		presets: new Map(),
		prefs: new Map(),
	};
	forms.set(id, made);
	return made;
}

/**
 * Row tables in a Map: the page session's memory (hosted links, the device page, the signed-out web, a Devices
 * runtime namespace's `runtimeMemory`) and the store's tests. Transactions run one after another.
 */
export function createMapTables(map: Map<string, unknown>) {
	let queue: Promise<unknown> = Promise.resolve();
	const tables: FormMemoryTables = {
		read: async (table, key) => [...rowsIn(map, key)[table].values()],
		async runIdsOldestFirst(key) {
			return [...rowsIn(map, key).runs.values()]
				.sort((a, b) => a.createdAt - b.createdAt)
				.map((row) => row.id);
		},
		async write(table, key, rows) {
			const target = rowsIn(map, key)[table];
			if (table === "prefs") target.clear();
			for (const row of rows) target.set(row.id, row);
		},
		async remove(table, key, ids) {
			const target = rowsIn(map, key)[table];
			if (table === "prefs") target.clear();
			for (const id of ids) target.delete(id);
		},
		transaction(work) {
			const run = queue.then(work, work);
			queue = run.catch(() => undefined);
			return run;
		},
	};
	return tables;
}

/**
 * `device`: the Dexie database `flow-like-form-memory`. `session`: `scope` (a per-page-session map such as a
 * Devices runtime namespace's `runtimeMemory(appId)`), else a module map that lives as long as the page.
 */
export const createFormMemoryStore: CreateFormMemoryStore = (mode, scope) =>
	createMemoryStoreOn(
		mode === "device"
			? dexieFormMemoryTables()
			: createMapTables(scope ?? pageSession),
		mode,
	);
