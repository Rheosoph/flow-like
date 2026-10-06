/*
 * Runs as this device saves them and reads them back (spec §5, PLAN §3.6–3.7). A record keeps the
 * run's copy as stored inputs (files as names and sizes, secrets as `{ $hidden: true }`), its status,
 * times, outcome and summary; never its answer or result. Read back, a run that was queued or sending
 * when the page closed is "Not started · the form was closed before its turn", and one still running
 * is unknown. Pure.
 */
import type {
	CopyValue,
	FormModel,
	HostCapabilities,
	RunEntry,
	RunOutcome,
	RunStatus,
	StoredRunRecord,
	StoredRunStatus,
} from "../contracts";
import { hiddenNames } from "../model/secrets";
import { fromStoredInputs, toStoredInputs } from "../model/stored";
import { isFileSlot, isReminder } from "../model/values";

/** The memory scope of session persistence (hosted, service, signed-out web): one per page session. */
export const SESSION_SCOPE = "session";

/** The scope records and the store's key use: the host's memory scope, else SESSION_SCOPE. */
export const memoryScopeOf = (host: Pick<HostCapabilities, "memoryScope">) =>
	host.memoryScope ?? SESSION_SCOPE;

const STORED_STATUS: Readonly<Record<RunStatus, StoredRunStatus>> = {
	sending: "sending",
	queued: "queued",
	starting: "running",
	asking: "running",
	running: "running",
	streaming: "running",
	done: "done",
	empty: "empty",
	failed: "failed",
	stopped: "stopped",
	notStarted: "notStarted",
	unknown: "running",
};

/** How a live status is saved: everything between dispatch and the end reads as `running`. */
export const storedStatusOf = (status: RunStatus): StoredRunStatus =>
	STORED_STATUS[status];

const outcomeToStore = (entry: RunEntry): RunOutcome | null =>
	entry.outcome?.kind === "noPlace" ? null : entry.outcome;

/**
 * A run as this device saves it. `noSave` is `prefs.noSave` ("Don't save {label}"): with the
 * secret words, `sensitive` and secret-looking values it decides which inputs are kept out.
 */
export function recordOf(
	entry: RunEntry,
	form: Pick<FormModel, "appId" | "eventId" | "fields">,
	scope: string,
	noSave: readonly string[] = [],
): StoredRunRecord {
	const hidden = hiddenNames(form.fields, entry.copy.values, noSave);
	return {
		version: 2,
		id: entry.id,
		scope,
		appId: form.appId,
		eventId: form.eventId,
		n: entry.n,
		createdAt: entry.createdAt,
		startedAt: entry.startedAt,
		endedAt: entry.endedAt,
		status: storedStatusOf(entry.status),
		outcome: outcomeToStore(entry),
		summary: entry.summary,
		failedAt: entry.failedAt,
		presetName: entry.copy.presetName,
		inputs: toStoredInputs(form.fields, entry.copy.values, hidden),
		perRun: entry.copy.perRun,
		replaced: entry.copy.replaced,
	};
}

const FORM_CLOSED: RunOutcome = { kind: "notStarted", reason: "formClosed" };
const UNKNOWN: RunOutcome = { kind: "unknown" };

interface ReadBack {
	readonly status: RunStatus;
	readonly outcome: RunOutcome | null;
}

const READ_BACK: Readonly<
	Record<StoredRunStatus, (record: StoredRunRecord) => ReadBack>
> = {
	queued: () => ({ status: "notStarted", outcome: FORM_CLOSED }),
	sending: () => ({ status: "notStarted", outcome: FORM_CLOSED }),
	running: () => ({ status: "unknown", outcome: UNKNOWN }),
	done: (record) => ({ status: "done", outcome: record.outcome }),
	empty: (record) => ({ status: "empty", outcome: record.outcome }),
	failed: (record) => ({ status: "failed", outcome: record.outcome }),
	stopped: (record) => ({ status: "stopped", outcome: record.outcome }),
	notStarted: (record) => ({
		status: "notStarted",
		outcome: record.outcome ?? FORM_CLOSED,
	}),
};

/** "Pick again" slots get ids unique per run: two runs that had the same file never share a slot id. */
function withRunReminderIds(
	runId: string,
	values: Readonly<Record<string, CopyValue>>,
): Readonly<Record<string, CopyValue>> {
	const rename = (item: unknown) =>
		isFileSlot(item) && isReminder(item)
			? { ...item, id: `${runId}:${item.id}` }
			: item;
	return Object.fromEntries(
		Object.entries(values).map(([name, value]) => [
			name,
			(Array.isArray(value) ? value.map(rename) : rename(value)) as CopyValue,
		]),
	);
}

/**
 * A saved run read back for this form (PLAN §3.7): its inputs fitted to today's fields (files as
 * "Pick again" reminders, kept-out values and object properties hidden), no output, nothing of this
 * session.
 */
export function entryOf(record: StoredRunRecord, form: FormModel): RunEntry {
	const { values } = fromStoredInputs(form.fields, record.inputs, {
		keepHidden: true,
	});
	const read = READ_BACK[record.status](record);
	return {
		id: record.id,
		n: record.n,
		origin: "history",
		status: read.status,
		createdAt: record.createdAt,
		startedAt: record.startedAt,
		endedAt: record.endedAt,
		copy: {
			values: withRunReminderIds(record.id, values),
			presetName: record.presetName,
			leftAsIs: false,
			perRun: record.perRun,
			replaced: record.replaced,
		},
		target: null,
		streamId: null,
		backendRunId: null,
		output: null,
		outcome: read.outcome,
		summary: record.summary,
		stopRequested: false,
		pendingSlotIds: [],
		waitingForPlace: false,
		failedAt: record.failedAt,
		unseenFailure: false,
	};
}

/**
 * This session's runs first (newest first), then the saved runs that are not of this session, newest
 * first. A run of this session keeps its in-memory entry when the store also has its record.
 */
export function mergeHistory(
	runs: readonly RunEntry[],
	records: readonly StoredRunRecord[],
	form: FormModel,
): readonly RunEntry[] {
	const session = runs.filter((run) => run.origin === "session");
	const known = new Set(session.map((run) => run.id));
	const history = records
		.filter((record) => !known.has(record.id))
		.map((record) => entryOf(record, form))
		.sort((a, b) => b.createdAt - a.createdAt || b.n - a.n);
	return [...session, ...history];
}

/** The next display number after every saved run (`prefs.nextRunNumber` is merged with it on load). */
export const nextNumberAfter = (records: readonly Pick<RunEntry, "n">[]) =>
	records.reduce((highest, record) => Math.max(highest, record.n), 0) + 1;
