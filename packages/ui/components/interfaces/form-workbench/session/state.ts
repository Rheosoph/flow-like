/*
 * The session's starting state and the readers every area of the reducer shares. Pure.
 */
import {
	BUSY_RUN_STATUSES,
	type FieldKey,
	type FieldValues,
	type FormPrefs,
	type FormSessionState,
	type InitialSessionState,
	type Preset,
	type RailState,
	type RunEntry,
	type RunStatus,
	type ViewState,
	type WorkbenchField,
} from "../contracts";
import { type FieldTarget, targets } from "../model/fields";
import { activePresetOf, perRunNamesOf } from "../model/markers";
import { capOf } from "../model/queue";
import { isSecretField, looksSecret } from "../model/secrets";
import { blockedNames } from "../model/validate";
import { type IsSecret, baselineOf } from "../model/values";

export const EMPTY_PREFS: FormPrefs = {
	version: 2,
	perRun: [],
	auto: [],
	introduced: false,
	offerAnswered: false,
	noSave: [],
	forgotten: {},
	nextRunNumber: 1,
	fieldSeenAt: {},
};

export function emptyRail(values: FieldValues): RailState {
	return {
		values,
		texts: {},
		problems: {},
		pressed: false,
		nextFiles: {},
		leftOut: {},
		dateAnchors: {},
		entryBegan: {},
		activePresetId: null,
		comparedRunId: null,
		tab: "inputs",
		filter: { query: "", chip: "all" },
		lastPressAt: null,
	};
}

export const INITIAL_VIEW: ViewState = {
	selectedRunId: null,
	pinnedRunId: null,
	pane: "inputs",
	overlay: null,
	focus: null,
	message: null,
	question: null,
	list: null,
	outputUnseen: false,
	stageFollowsNewest: true,
};

/**
 * The form at its defaults, before memory loads: no runs, no preset, nothing per run. Hosted and
 * service pages know their target already (`fixedTarget`); the app host resolves it per session.
 */
export const initialSessionState: InitialSessionState = (form) => {
	const target = form.host.fixedTarget;
	return {
		form,
		layout: null,
		rail: emptyRail(baselineOf(form.fields, null)),
		runs: [],
		view: INITIAL_VIEW,
		memory: { loaded: false, prefs: EMPTY_PREFS, presets: [] },
		queue: {
			target,
			tierLimit: null,
			cap: capOf(target, null),
			hold: null,
			retryAt: null,
		},
		announcement: null,
		undo: null,
		seq: 0,
	};
};

// ─── Readers ────────────────────────────────────────────────────────────────

export function fieldByName(
	state: Pick<FormSessionState, "form">,
	name: string,
): WorkbenchField | null {
	return state.form.fields.find((field) => field.name === name) ?? null;
}

/** A field or object property by its FieldKey, with its object's name. */
export function targetOf(
	state: Pick<FormSessionState, "form">,
	key: FieldKey,
): FieldTarget | null {
	return (
		targets(state.form.fields).find((target) => target.key === key) ?? null
	);
}

/** Fields per run now (setting and series), only those the form still has. */
export function livePerRun(
	state: Pick<FormSessionState, "form" | "memory">,
): readonly string[] {
	const names = new Set(state.form.fields.map((field) => field.name));
	return perRunNamesOf(state).filter((name) => names.has(name));
}

export function activePreset(
	state: Pick<FormSessionState, "rail" | "memory">,
): Preset | null {
	return activePresetOf(state);
}

/** Secret fields (`sensitive`, "Don't save", secret words) and secret-looking values (spec M6). */
export function secretTest(state: Pick<FormSessionState, "memory">): IsSecret {
	const noSave = state.memory.prefs.noSave;
	return (field, value) =>
		isSecretField(field, noSave) || (value !== undefined && looksSecret(value));
}

/** FlowPath fields this host cannot fill: no ↵ stop, never a cursor target. */
export const blockedOf = (state: Pick<FormSessionState, "form">) =>
	blockedNames(state.form.fields, state.form.host);

/** Rail and stage side by side (also before the shell has measured the box). */
export const isSplit = (state: Pick<FormSessionState, "layout">) =>
	state.layout?.split ?? true;

export function runById(
	state: Pick<FormSessionState, "runs">,
	id: string | null,
): RunEntry | null {
	if (id === null) return null;
	return state.runs.find((run) => run.id === id) ?? null;
}

/** The newest run of this window, whatever its state. */
export function newestSessionRun(
	state: Pick<FormSessionState, "runs">,
): RunEntry | null {
	return state.runs.find((run) => run.origin === "session") ?? null;
}

/** The newest run of this window that ran or still waits to: a run taken out before it began never ran. */
export function newestEnteredRun(
	state: Pick<FormSessionState, "runs">,
): RunEntry | null {
	return (
		state.runs.find(
			(run) => run.origin === "session" && run.status !== "notStarted",
		) ?? null
	);
}

const STARTED: ReadonlySet<RunStatus> = new Set<RunStatus>([
	"starting",
	"asking",
	"running",
	"streaming",
	"done",
	"empty",
	"failed",
	"stopped",
	"unknown",
]);

/** The newest run that started (not queued, sending or taken out before it began). */
export function newestStartedRun(runs: readonly RunEntry[]): RunEntry | null {
	return runs.find((run) => STARTED.has(run.status)) ?? null;
}

export const isBusy = (run: Pick<RunEntry, "status">) =>
	BUSY_RUN_STATUSES.includes(run.status);

/** Runs that wait for a place or for their files. */
export const isWaiting = (run: Pick<RunEntry, "status">) =>
	run.status === "queued" || run.status === "sending";

export const busyCount = (state: Pick<FormSessionState, "runs">) =>
	state.runs.filter(isBusy).length;

/** What a streamed chunk changes on a live run: its output and the summary read from it. */
const STREAMED: ReadonlySet<string> = new Set<keyof RunEntry>([
	"output",
	"summary",
]);

function sameButStreamed(a: RunEntry, b: RunEntry) {
	if (a === b) return true;
	const keys = Object.keys(b) as (keyof RunEntry)[];
	return (
		keys.length === Object.keys(a).length &&
		keys.every((key) => STREAMED.has(key) || a[key] === b[key])
	);
}

/**
 * Two states that differ at most in what live runs streamed: the rail's inputs and the dock show nothing of it, so
 * they need not render again for every chunk of a streamed answer.
 */
export function sameButStreamedOutput(
	a: FormSessionState,
	b: FormSessionState,
): boolean {
	if (a === b) return true;
	const keys = Object.keys(b) as (keyof FormSessionState)[];
	if (keys.length !== Object.keys(a).length) return false;
	return keys.every((key) =>
		key === "runs"
			? a.runs.length === b.runs.length &&
				a.runs.every((run, index) => sameButStreamed(run, b.runs[index]))
			: a[key] === b[key],
	);
}
