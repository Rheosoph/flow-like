/*
 * One reduction step: the clock, the effects asked for so far, the sequence counter for focus,
 * messages, announcements and Undo, and the slots this step picked, aborted or released. Local to
 * one `reduceSession` call, so the reducer stays pure. Small state helpers every area shares.
 */
import {
	type DockMessage,
	type DockMessageEntry,
	FIELD_KEY_SEPARATOR,
	FORM_LIMITS,
	type FieldKey,
	type FieldProblem,
	type FocusTarget,
	type FormPrefs,
	type FormSessionState,
	type Preset,
	type RailState,
	type RunEntry,
	type SessionClock,
	type SessionCommand,
	type SessionEffect,
	type SessionInput,
	type UndoEntry,
	type ViewState,
} from "../contracts";
import { memoryScopeOf, recordOf } from "./history";

export type CommandType = SessionCommand["type"];
export type CommandOf<T extends CommandType> = Extract<
	SessionCommand,
	{ readonly type: T }
>;
/** One handler per command type; each area of the reducer exports a slice of the table. */
export type CommandHandlers<T extends CommandType = CommandType> = {
	readonly [K in T]: (
		state: FormSessionState,
		command: CommandOf<K>,
		tx: Tx,
	) => FormSessionState;
};

export type InputType = SessionInput["type"];
export type InputOf<T extends InputType> = Extract<
	SessionInput,
	{ readonly type: T }
>;
export type InputHandlers<T extends InputType = InputType> = {
	readonly [K in T]: (
		state: FormSessionState,
		input: InputOf<K>,
		tx: Tx,
	) => FormSessionState;
};

export interface Tx {
	readonly clock: SessionClock;
	readonly effects: SessionEffect[];
	/** Slot ids of files picked in this step (the runtime holds their File objects already). */
	readonly picked: string[];
	/** Slots whose upload this step aborted or whose File it released outright (leaving the form). */
	readonly aborted: Set<string>;
	readonly released: Set<string>;
	seq: number;
}

export function createTx(
	state: Pick<FormSessionState, "seq">,
	clock: SessionClock,
): Tx {
	return {
		clock,
		effects: [],
		picked: [],
		aborted: new Set(),
		released: new Set(),
		seq: state.seq,
	};
}

export function nextSeq(tx: Tx) {
	tx.seq += 1;
	return tx.seq;
}

export function emit(tx: Tx, effect: SessionEffect) {
	tx.effects.push(effect);
}

// ─── Parts of the state ─────────────────────────────────────────────────────

export function withRail(
	state: FormSessionState,
	patch: Partial<RailState>,
): FormSessionState {
	return { ...state, rail: { ...state.rail, ...patch } };
}

export function withView(
	state: FormSessionState,
	patch: Partial<ViewState>,
): FormSessionState {
	return { ...state, view: { ...state.view, ...patch } };
}

/** `prefs` with a change, saved on this device. */
export function withPrefs(
	state: FormSessionState,
	tx: Tx,
	patch: Partial<FormPrefs>,
): FormSessionState {
	const prefs: FormPrefs = { ...state.memory.prefs, ...patch };
	emit(tx, { type: "persistPrefs", prefs });
	return { ...state, memory: { ...state.memory, prefs } };
}

export function withPresets(
	state: FormSessionState,
	presets: readonly Preset[],
): FormSessionState {
	return { ...state, memory: { ...state.memory, presets } };
}

export function withFocus(
	state: FormSessionState,
	tx: Tx,
	target: FocusTarget,
): FormSessionState {
	return withView(state, { focus: { seq: nextSeq(tx), target } });
}

export const fieldFocus = (
	key: FieldKey,
	select = false,
	scrollOnly = false,
): FocusTarget => ({ kind: "field", key, select, scrollOnly });

// ─── Dock messages and Undo ────────────────────────────────────────────────

/** How long a message stays: plain ones FORM_LIMITS.messageMs; with Undo or an action until the next edit or run. */
export type MessageLife = "plain" | "undo" | "action";

function messageEntry(
	tx: Tx,
	seq: number,
	message: DockMessage,
	life: MessageLife,
): DockMessageEntry {
	const plain = life === "plain";
	if (plain)
		emit(tx, {
			type: "expireMessage",
			seq,
			afterMs: FORM_LIMITS.messageMs,
		});
	return {
		seq,
		message,
		undo: life === "undo",
		expiresAt: plain ? tx.clock.now + FORM_LIMITS.messageMs : null,
	};
}

export function withMessage(
	state: FormSessionState,
	tx: Tx,
	message: DockMessage,
	life: MessageLife = "plain",
): FormSessionState {
	return withView(state, {
		message: messageEntry(tx, nextSeq(tx), message, life),
	});
}

export interface UndoParts {
	readonly kind: UndoEntry["kind"];
	/** The state before the change Undo restores (values, next files, active preset). */
	readonly before: FormSessionState;
	readonly deletedPreset?: Preset | null;
}

/** A reported change with Undo: the message and the one-level Undo share a sequence number. */
export function withUndoMessage(
	state: FormSessionState,
	tx: Tx,
	message: DockMessage,
	parts: UndoParts,
): FormSessionState {
	const seq = nextSeq(tx);
	const undo: UndoEntry = {
		seq,
		kind: parts.kind,
		values: parts.before.rail.values,
		nextFiles: parts.before.rail.nextFiles,
		activePresetId: parts.before.rail.activePresetId,
		deletedPreset: parts.deletedPreset ?? null,
	};
	const entry = messageEntry(tx, seq, message, "undo");
	return { ...withView(state, { message: entry }), undo };
}

/** An edit or a run: Undo and the dock message (with Undo, an action, or plain) go. */
export function clearEdits(state: FormSessionState): FormSessionState {
	if (state.undo === null && state.view.message === null) return state;
	return { ...withView(state, { message: null }), undo: null };
}

// ─── Problems and typed text by key ─────────────────────────────────────────

const inKey = (key: FieldKey) => (candidate: string) =>
	candidate === key || candidate.startsWith(`${key}${FIELD_KEY_SEPARATOR}`);

function without<T>(
	record: Readonly<Record<string, T>>,
	key: FieldKey,
): Readonly<Record<string, T>> {
	const covered = inKey(key);
	if (!Object.keys(record).some(covered)) return record;
	return Object.fromEntries(
		Object.entries(record).filter(([name]) => !covered(name)),
	);
}

/** The key's message (and its properties' messages for an object) goes. */
export function withoutProblem(
	state: FormSessionState,
	key: FieldKey,
): FormSessionState {
	const problems = without(state.rail.problems, key);
	return problems === state.rail.problems
		? state
		: withRail(state, { problems });
}

export function withProblem(
	state: FormSessionState,
	key: FieldKey,
	problem: FieldProblem | null,
): FormSessionState {
	if (problem === null) return withoutProblem(state, key);
	return withRail(state, {
		problems: { ...state.rail.problems, [key]: problem },
	});
}

export function withoutText(
	state: FormSessionState,
	key: FieldKey,
): FormSessionState {
	const texts = without(state.rail.texts, key);
	return texts === state.rail.texts ? state : withRail(state, { texts });
}

// ─── Runs ───────────────────────────────────────────────────────────────────

export function mapRun(
	state: FormSessionState,
	id: string,
	update: (run: RunEntry) => RunEntry,
): FormSessionState {
	let changed = false;
	const runs = state.runs.map((run) => {
		if (run.id !== id) return run;
		const next = update(run);
		changed = changed || next !== run;
		return next;
	});
	return changed ? { ...state, runs } : state;
}

/** Saves a run of this session as it is now (every status change). */
export function persistRun(state: FormSessionState, tx: Tx, run: RunEntry) {
	const scope = memoryScopeOf(state.form.host);
	const record = recordOf(run, state.form, scope, state.memory.prefs.noSave);
	emit(tx, { type: "persistRun", record });
}

/** Updates a run and saves it. */
export function changeRun(
	state: FormSessionState,
	tx: Tx,
	id: string,
	update: (run: RunEntry) => RunEntry,
): FormSessionState {
	const next = mapRun(state, id, update);
	const run = next.runs.find((candidate) => candidate.id === id);
	if (next !== state && run) persistRun(next, tx, run);
	return next;
}
