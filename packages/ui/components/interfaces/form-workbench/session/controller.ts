/*
 * The session runtime (PLAN §3.2, §3.7). A controller holds one form session's state, runs the
 * reducer on every message and carries out the effects it asks for, feeding what happens back as
 * inputs. It owns everything with side effects: File objects by slot id, uploads, dispatched runs
 * and their accumulators, timers, the memory store, the target and tier lookups, and navigation.
 * React reads it through `useSyncExternalStore`; registry.ts keeps it across remounts.
 *
 * Attached (a mount holds it): the controller holds a no-op subscription on every live stream, so
 * the engine's background card leaves them out. Detached for FORM_LIMITS.leaveGraceMs without a
 * new mount, the form is left: the reducer takes waiting runs out (`detached`), the streams lose
 * their subscription and show in the card, and the controller ends once its last run settled.
 */
import { resolveExecutionTarget } from "../../../../lib/execution-target";
import type { IEventState } from "../../../../state/backend-state/event-state";
import type { IHelperState } from "../../../../state/backend-state/helper-state";
import type { IUserState } from "../../../../state/backend-state/user-state";
import {
	type ExecutionTarget,
	FORM_LIMITS,
	type FileMode,
	type FormMemoryKey,
	type FormMemoryStore,
	type FormModel,
	type FormSessionActions,
	type FormSessionState,
	type InitialSessionState,
	type LoadedMemory,
	type NavigateIntent,
	type PickedFile,
	type ReduceSession,
	type RunOutcome,
	type SessionClock,
	type SessionCommand,
	type SessionEffect,
	type SessionInput,
	type SessionMessage,
} from "../contracts";
import {
	OUTPUT_FRAME_MS,
	type RespondInteraction,
	type RunHandle,
	type RuntimeEngine,
	type ScheduleFrame,
	answerInteraction,
	dispatchRun,
	dropStream,
	holdStream,
	releaseStream,
	stopRun,
} from "./dispatch-run";
import { memoryScopeOf } from "./history";
import { NO_FILE_ERROR, type Upload, startUpload } from "./uploads";

/** What the runtime needs of the host's backend. */
export interface RuntimeBackend {
	readonly eventState: Pick<
		IEventState,
		"cancelExecution" | "alwaysRemote" | "prerunEvent"
	>;
	readonly helperState: Pick<
		IHelperState,
		"fileToUrl" | "filesToTemporaryFiles"
	>;
	readonly userState: Pick<IUserState, "getPricing">;
}

export interface RuntimeTimers {
	setTimeout(callback: () => void, ms: number): unknown;
	clearTimeout(handle: unknown): void;
}

export interface SessionControllerOptions {
	readonly engine: RuntimeEngine;
	readonly backend: RuntimeBackend;
	readonly form: FormModel;
	/** The reducer and the starting state: S-STATE's in the product, scripted ones in tests. */
	readonly reduce: ReduceSession;
	readonly initial: InitialSessionState;
	readonly memory: FormMemoryStore;
	readonly navigate?: (intent: NavigateIntent) => void;
	readonly now?: () => number;
	readonly timers?: RuntimeTimers;
	readonly respond?: RespondInteraction;
}

const EMPTY_MEMORY: LoadedMemory = { prefs: null, presets: [], runs: [] };
/** A retry timer never fires before the reducer's `retryAt`. */
const RETRY_SLACK_MS = 50;
/** Waits before asking again whether the engine finished a settled run's stream. */
const DROP_RETRY_MS: readonly number[] = [0, 50, 250, 1000];

const DEFAULT_TIMERS: RuntimeTimers = {
	setTimeout: (callback, ms) => globalThis.setTimeout(callback, ms),
	clearTimeout: (handle) =>
		globalThis.clearTimeout(handle as ReturnType<typeof setTimeout>),
};

const NO_NAVIGATION = () => {};

let controllerSeq = 0;

/** The local calendar day, `YYYY-MM-DD`. */
export function localDay(at: Date): string {
	const pad = (value: number) => String(value).padStart(2, "0");
	return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())}`;
}

export const sessionClock = (now: number): SessionClock => ({
	now,
	today: localDay(new Date(now)),
});

/** The account's cloud runs at once (`max_concurrent_executions`, −1 for none); null when unknown. */
export async function tierLimitOf(
	userState: Pick<IUserState, "getPricing">,
): Promise<number | null> {
	try {
		const pricing = await userState.getPricing();
		const limit =
			pricing?.tiers?.[pricing.current_tier]?.max_concurrent_executions;
		return typeof limit === "number" && Number.isFinite(limit) ? limit : null;
	} catch {
		return null;
	}
}

/** A message by its type only: its values may hold secrets. */
const messageType = (message: SessionMessage) =>
	message.kind === "command"
		? `command ${message.command.type}`
		: `input ${message.input.type}`;

/** What makes a host's form the same form for the session: content, node, host and viewer. */
const formIdentity = (form: FormModel) =>
	JSON.stringify([
		form.contentKey,
		form.nodeId,
		form.host,
		form.viewer.locale,
		form.viewer.mac,
		form.viewer.decimalSign,
	]);

// ─── Actions ────────────────────────────────────────────────────────────────

type Send = (command: SessionCommand) => void;
type KeepFiles = (files: readonly File[]) => readonly PickedFile[];

const railActions = (
	send: Send,
): Pick<
	FormSessionActions,
	| "setValue"
	| "setText"
	| "commitText"
	| "blurField"
	| "resetField"
	| "resetAll"
	| "enter"
	| "railKey"
	| "setFilter"
	| "setRailTab"
> => ({
	setValue: (key, value, how) =>
		send(
			how
				? { type: "setValue", key, value, how }
				: { type: "setValue", key, value },
		),
	setText: (key, text) => send({ type: "setText", key, text }),
	commitText: (key) => send({ type: "commitText", key }),
	blurField: (key) => send({ type: "blurField", key }),
	resetField: (key) => send({ type: "resetField", key }),
	resetAll: () => send({ type: "resetAll" }),
	enter: (fromKey) => send({ type: "enter", fromKey }),
	railKey: () => send({ type: "railKey" }),
	setFilter: (filter) => send({ type: "setFilter", filter }),
	setRailTab: (tab) => send({ type: "setRailTab", tab }),
});

const fileActions = (
	send: Send,
	keepFiles: KeepFiles,
): Pick<
	FormSessionActions,
	| "pickFiles"
	| "removeFile"
	| "retryFile"
	| "removeNextFile"
	| "clearNextFiles"
	| "addLeftOut"
> => ({
	pickFiles: (name, files, mode) =>
		send({ type: "pickFiles", name, files: keepFiles(files), mode }),
	removeFile: (name, slotId) => send({ type: "removeFile", name, slotId }),
	retryFile: (slotId) => send({ type: "retryFile", slotId }),
	removeNextFile: (name, slotId) =>
		send({ type: "removeNextFile", name, slotId }),
	clearNextFiles: (name) => send({ type: "clearNextFiles", name }),
	addLeftOut: (name) => send({ type: "addLeftOut", name }),
});

const runActions = (
	send: Send,
): Pick<
	FormSessionActions,
	| "run"
	| "runAgain"
	| "stop"
	| "removeFromQueue"
	| "clearQueue"
	| "resumeQueue"
	| "useInputs"
	| "removeRun"
	| "respondInteraction"
> => ({
	run: (options) =>
		send({
			type: "run",
			leaveAsIs: options?.leaveAsIs ?? false,
			from: options?.from ?? "button",
		}),
	runAgain: (runId) => send({ type: "runAgain", runId }),
	stop: (runId) => send({ type: "stop", runId }),
	removeFromQueue: (runId) => send({ type: "removeFromQueue", runId }),
	clearQueue: () => send({ type: "clearQueue" }),
	resumeQueue: () => send({ type: "resumeQueue" }),
	useInputs: (runId) => send({ type: "useInputs", runId }),
	removeRun: (runId) => send({ type: "removeRun", runId }),
	respondInteraction: (runId, interactionId, value) =>
		send({ type: "respondInteraction", runId, interactionId, value }),
});

const viewActions = (
	send: Send,
): Pick<
	FormSessionActions,
	| "selectRun"
	| "pinRun"
	| "setPane"
	| "openOverlay"
	| "closeOverlay"
	| "openList"
	| "moveList"
	| "closeList"
	| "dismissMessage"
	| "focusHandled"
	| "markSeen"
	| "setLayout"
> => ({
	selectRun: (runId, how) => send({ type: "selectRun", runId, how }),
	pinRun: (runId) => send({ type: "pinRun", runId }),
	setPane: (pane) => send({ type: "setPane", pane }),
	openOverlay: (overlay) => send({ type: "openOverlay", overlay }),
	closeOverlay: () => send({ type: "closeOverlay" }),
	openList: (kind, key) => send({ type: "openList", kind, key }),
	moveList: (delta) => send({ type: "moveList", delta }),
	closeList: () => send({ type: "closeList" }),
	dismissMessage: () => send({ type: "dismissMessage" }),
	focusHandled: (seq) => send({ type: "focusHandled", seq }),
	markSeen: (runId) => send({ type: "markSeen", runId }),
	setLayout: (layout) => send({ type: "setLayout", layout }),
});

const memoryActions = (
	send: Send,
): Pick<
	FormSessionActions,
	| "setPerRun"
	| "uncheckAllPerRun"
	| "perRunFilesAndDates"
	| "answerQuestion"
	| "forgetRecent"
	| "dontSave"
	| "applyPreset"
	| "savePreset"
	| "updatePreset"
	| "deletePreset"
	| "resetToPreset"
	| "undo"
> => ({
	setPerRun: (name, on) => send({ type: "setPerRun", name, on }),
	uncheckAllPerRun: () => send({ type: "uncheckAllPerRun" }),
	perRunFilesAndDates: () => send({ type: "perRunFilesAndDates" }),
	answerQuestion: (yes) => send({ type: "answerQuestion", yes }),
	forgetRecent: (name, value) => send({ type: "forgetRecent", name, value }),
	dontSave: (name) => send({ type: "dontSave", name }),
	applyPreset: (presetId) => send({ type: "applyPreset", presetId }),
	savePreset: (draft) => send({ type: "savePreset", draft }),
	updatePreset: (presetId) => send({ type: "updatePreset", presetId }),
	deletePreset: (presetId) => send({ type: "deletePreset", presetId }),
	resetToPreset: () => send({ type: "resetToPreset" }),
	undo: () => send({ type: "undo" }),
});

/** Every UI action as a session command; picked files are kept by slot id first. */
export const sessionActions = (
	send: Send,
	keepFiles: KeepFiles,
): FormSessionActions => ({
	...railActions(send),
	...fileActions(send, keepFiles),
	...runActions(send),
	...viewActions(send),
	...memoryActions(send),
});

// ─── The controller ─────────────────────────────────────────────────────────

type EffectRunners = {
	readonly [K in SessionEffect["type"]]: (
		effect: Extract<SessionEffect, { readonly type: K }>,
	) => void;
};

type MemoryWrite = (
	store: FormMemoryStore,
	key: FormMemoryKey,
) => Promise<void>;

export class FormSessionController {
	readonly id: string;
	/** The engine subscriber id of this controller's no-op subscriptions. */
	readonly subscriberId: string;
	readonly actions: FormSessionActions;
	private state: FormSessionState;
	private backend: RuntimeBackend;
	private navigateTo: (intent: NavigateIntent) => void;
	private readonly engine: RuntimeEngine;
	private readonly reduce: ReduceSession;
	private readonly memory: FormMemoryStore;
	private readonly memoryKey: FormMemoryKey;
	private readonly now: () => number;
	private readonly timers: RuntimeTimers;
	private readonly respond: RespondInteraction | undefined;
	private readonly listeners = new Set<() => void>();
	private readonly disposeListeners = new Set<() => void>();
	private readonly files = new Map<string, File>();
	private readonly uploads = new Map<string, Upload>();
	private readonly runs = new Map<string, RunHandle>();
	private readonly scheduled = new Set<unknown>();
	private effects: SessionEffect[] = [];
	private draining = false;
	private changed = false;
	private disposePending = false;
	private attachCount = 0;
	private started = false;
	private left = false;
	private disposed = false;
	private holding = false;
	private leaveTimer: unknown = null;
	private retryTimer: unknown = null;
	private pendingNavigation: NavigateIntent[] = [];
	private target: Promise<ExecutionTarget> | null = null;
	private writes: Promise<void> = Promise.resolve();
	private slotSeq = 0;

	private readonly runners: EffectRunners = {
		upload: (effect) => this.upload(effect.slotId, effect.mode),
		abortUpload: (effect) => this.abortUpload(effect.slotId),
		releaseFiles: (effect) => this.releaseFiles(effect.slotIds),
		dispatchRun: (effect) => this.startRun(effect.runId),
		stopRun: (effect) => this.stopRun(effect.runId),
		persistRun: (effect) => this.write((store) => store.putRun(effect.record)),
		deleteRun: (effect) =>
			this.write((store, key) => store.deleteRun(key, effect.id)),
		hideField: (effect) =>
			this.write((store, key) => store.hideField(key, effect.name)),
		persistPrefs: (effect) =>
			this.write((store, key) => store.putPrefs(key, effect.prefs)),
		persistPreset: (effect) =>
			this.write((store, key) => store.putPreset(key, effect.preset)),
		deletePreset: (effect) =>
			this.write((store, key) => store.deletePreset(key, effect.presetId)),
		expireMessage: (effect) =>
			this.later(effect.afterMs, () =>
				this.report({ type: "messageExpired", seq: effect.seq }),
			),
		scheduleRetry: (effect) => this.scheduleRetry(effect.afterMs),
		respondInteraction: (effect) =>
			this.answer(effect.runId, effect.interactionId, effect.value),
	};

	/** Live output of every run is reported on the session's own timers, at most once a frame. */
	private readonly frame: ScheduleFrame = (callback) => {
		const timer = this.timers.setTimeout(callback, OUTPUT_FRAME_MS);
		return () => this.timers.clearTimeout(timer);
	};

	constructor(options: SessionControllerOptions) {
		controllerSeq += 1;
		this.id = `form-session-${controllerSeq}`;
		this.subscriberId = `form-workbench:${this.id}`;
		this.engine = options.engine;
		this.backend = options.backend;
		this.reduce = options.reduce;
		this.memory = options.memory;
		this.now = options.now ?? Date.now;
		this.timers = options.timers ?? DEFAULT_TIMERS;
		this.respond = options.respond;
		this.navigateTo = options.navigate ?? NO_NAVIGATION;
		this.memoryKey = {
			scope: memoryScopeOf(options.form.host),
			appId: options.form.appId,
			eventId: options.form.eventId,
		};
		this.state = options.initial(options.form, sessionClock(this.now()));
		this.actions = sessionActions(
			(command) => this.dispatch({ kind: "command", command }),
			(files) => this.keepFiles(files),
		);
	}

	getState = (): FormSessionState => this.state;

	subscribe = (listener: () => void): (() => void) => {
		this.listeners.add(listener);
		return () => {
			this.listeners.delete(listener);
		};
	};

	isAttached(): boolean {
		return this.attachCount > 0;
	}

	isDisposed(): boolean {
		return this.disposed;
	}

	onDispose(listener: () => void): () => void {
		this.disposeListeners.add(listener);
		return () => {
			this.disposeListeners.delete(listener);
		};
	}

	/** The host's latest backend and navigate; runs already sent keep the backend they were sent with. */
	update(parts: {
		readonly backend?: RuntimeBackend;
		readonly navigate?: (intent: NavigateIntent) => void;
	}): void {
		if (parts.backend) this.backend = parts.backend;
		if (parts.navigate) this.navigateTo = parts.navigate;
	}

	/** The host's form again: `formChanged` only when its content, node, host or viewer differ. */
	setForm(form: FormModel): void {
		if (formIdentity(form) === formIdentity(this.state.form)) return;
		this.report({ type: "formChanged", form });
	}

	dispatch(message: SessionMessage): void {
		if (this.disposed) return;
		this.reduceWith(message);
		if (!this.draining) this.drain();
	}

	/** A mount holds the controller (reference-counted); the first one starts memory and target. */
	attach(): void {
		if (this.disposed) return;
		this.attachCount += 1;
		if (this.attachCount > 1) return;
		this.clearLeaveTimer();
		this.left = false;
		this.holdStreams();
		this.start();
		this.report({ type: "attached" });
		this.flushNavigation();
	}

	/** A mount let go; without a new one within the grace, the form is left. */
	detach(): void {
		if (this.attachCount === 0) return;
		this.attachCount -= 1;
		if (this.attachCount > 0 || this.disposed) return;
		this.clearLeaveTimer();
		this.leaveTimer = this.timers.setTimeout(
			() => this.leave(),
			FORM_LIMITS.leaveGraceMs,
		);
	}

	// ─── Dispatching ────────────────────────────────────────────────────────

	private report(input: SessionInput): void {
		this.dispatch({ kind: "input", input });
	}

	private reduceWith(message: SessionMessage): void {
		try {
			const step = this.reduce(this.state, message, sessionClock(this.now()));
			if (step.state !== this.state) {
				this.state = step.state;
				this.changed = true;
			}
			this.effects.push(...step.effects);
		} catch (error) {
			console.error(
				`[form-workbench] the session could not take ${messageType(message)}`,
				error,
			);
		}
	}

	/** Effects run in order; inputs reported meanwhile reduce at once and queue their effects behind. */
	private drain(): void {
		this.draining = true;
		try {
			for (
				let effect = this.effects.shift();
				effect !== undefined;
				effect = this.effects.shift()
			)
				this.run(effect);
		} finally {
			this.draining = false;
		}
		this.notify();
		if (this.disposePending) {
			this.disposePending = false;
			this.disposeWhenIdle();
		}
	}

	private run(effect: SessionEffect): void {
		try {
			(this.runners[effect.type] as (effect: SessionEffect) => void)(effect);
		} catch (error) {
			console.error(`[form-workbench] effect ${effect.type} failed`, error);
		}
	}

	private notify(): void {
		if (!this.changed) return;
		this.changed = false;
		for (const listener of [...this.listeners]) listener();
	}

	// ─── Start, memory and target ───────────────────────────────────────────

	private start(): void {
		if (this.started) return;
		this.started = true;
		void this.loadMemory();
		this.resolveTarget().catch((error: unknown) =>
			console.warn("[form-workbench] the run target could not be read", error),
		);
	}

	private async loadMemory(): Promise<void> {
		let memory = EMPTY_MEMORY;
		try {
			memory = await this.memory.load(this.memoryKey);
		} catch (error) {
			console.warn("[form-workbench] saved runs could not be read", error);
		}
		this.report({ type: "memoryLoaded", memory });
	}

	private targetOf(): Promise<ExecutionTarget> {
		this.target ??= this.lookUpTarget();
		return this.target;
	}

	private lookUpTarget(): Promise<ExecutionTarget> {
		const { appId, eventId, host } = this.state.form;
		if (host.fixedTarget) return Promise.resolve(host.fixedTarget);
		return resolveExecutionTarget(this.backend, appId, eventId);
	}

	/**
	 * The target decides the cap: the app host resolves it, where the cloud's cap is the tier's limit;
	 * hosted and service pages report their fixed target (the tier stays unknown there).
	 */
	private async resolveTarget(): Promise<void> {
		const target = await this.targetOf();
		this.report({ type: "targetResolved", target, tierLimit: null });
		if (target !== "remote" || this.state.form.host.fixedTarget) return;
		const tierLimit = await tierLimitOf(this.backend.userState);
		if (tierLimit !== null)
			this.report({ type: "targetResolved", target, tierLimit });
	}

	private write(op: MemoryWrite): void {
		const { memory, memoryKey } = this;
		this.writes = this.writes
			.then(() => op(memory, memoryKey))
			.catch((error: unknown) =>
				console.warn(
					"[form-workbench] the form memory could not be saved",
					error,
				),
			);
	}

	// ─── Files ──────────────────────────────────────────────────────────────

	private keepFiles(files: readonly File[]): readonly PickedFile[] {
		if (this.disposed) return [];
		return files.map((file) => {
			this.slotSeq += 1;
			const slotId = `${this.id}:file-${this.slotSeq}`;
			this.files.set(slotId, file);
			return { slotId, name: file.name, size: file.size, type: file.type };
		});
	}

	private upload(slotId: string, mode: FileMode): void {
		this.abortUpload(slotId);
		const file = this.files.get(slotId);
		if (!file) {
			this.report({ type: "uploadFailed", slotId, error: NO_FILE_ERROR });
			return;
		}
		const { appId, eventId } = this.state.form;
		const upload = startUpload({
			slotId,
			file,
			mode,
			appId,
			eventId,
			target: () => this.targetOf(),
			helper: this.backend.helperState,
			report: (input) => this.report(input),
		});
		this.uploads.set(slotId, upload);
		void upload.done.then(() => {
			if (this.uploads.get(slotId) === upload) this.uploads.delete(slotId);
		});
	}

	private abortUpload(slotId: string): void {
		const upload = this.uploads.get(slotId);
		if (!upload) return;
		this.uploads.delete(slotId);
		upload.abort();
	}

	private releaseFiles(slotIds: readonly string[]): void {
		for (const slotId of slotIds) {
			this.abortUpload(slotId);
			this.files.delete(slotId);
		}
	}

	// ─── Runs ───────────────────────────────────────────────────────────────

	private startRun(runId: string): void {
		const run = this.state.runs.find((entry) => entry.id === runId);
		if (!run || this.runs.has(runId)) return;
		const handle = dispatchRun({
			engine: this.engine,
			eventState: this.backend.eventState,
			helper: this.backend.helperState,
			form: this.state.form,
			run,
			fileOf: (slotId) => this.files.get(slotId),
			report: (input) => this.report(input),
			navigate: (intents) => this.navigateAll(intents),
			onStream: (started) => this.onStream(started),
			settle: (settled, outcome) => this.settled(settled, outcome),
			frame: this.frame,
		});
		this.runs.set(runId, handle);
	}

	private stopRun(runId: string): void {
		const handle = this.runs.get(runId);
		if (handle) stopRun(handle);
	}

	private onStream(handle: RunHandle): void {
		if (this.holding)
			holdStream(this.engine, handle.streamId, this.subscriberId);
	}

	/** The outcome first, then the stream goes (so the card never shows a finished run), then the record. */
	private settled(handle: RunHandle, outcome: RunOutcome): void {
		if (this.runs.get(handle.runId) === handle) this.runs.delete(handle.runId);
		if (handle.dispatched) this.drop(handle.streamId, 0);
		this.report({ type: "runSettled", runId: handle.runId, outcome });
		this.disposeWhenIdle();
	}

	private drop(streamId: string, attempt: number): void {
		if (dropStream(this.engine, streamId, this.subscriberId)) return;
		const wait = DROP_RETRY_MS[attempt];
		if (wait === undefined) return;
		this.timers.setTimeout(() => this.drop(streamId, attempt + 1), wait);
	}

	private answer(runId: string, interactionId: string, value: unknown): void {
		const handle = this.runs.get(runId);
		if (!handle) {
			this.answerFailed(
				runId,
				interactionId,
				`${runId} ended before its question ${interactionId} was answered`,
			);
			return;
		}
		answerInteraction(
			handle,
			interactionId,
			value,
			(input) => this.report(input),
			this.respond,
		).catch((error: unknown) =>
			this.answerFailed(runId, interactionId, String(error)),
		);
	}

	private answerFailed(runId: string, interactionId: string, error: string) {
		console.warn(
			`[form-workbench] the answer to ${interactionId} could not be sent`,
			error,
		);
		this.report({ type: "interactionFailed", runId, interactionId, error });
	}

	// ─── Streams and navigation ─────────────────────────────────────────────

	private liveHandles(): RunHandle[] {
		return [...this.runs.values()].filter(
			(handle) => handle.dispatched && !handle.settled,
		);
	}

	private holdStreams(): void {
		this.holding = true;
		for (const handle of this.liveHandles())
			holdStream(this.engine, handle.streamId, this.subscriberId);
	}

	private releaseStreams(): void {
		this.holding = false;
		for (const handle of this.liveHandles())
			releaseStream(this.engine, handle.streamId, this.subscriberId);
	}

	/** Navigation of a run: now while a mount holds the form, after a quick remount, never once left. */
	private navigateAll(intents: readonly NavigateIntent[]): void {
		if (this.attachCount === 0) {
			if (!this.left) this.pendingNavigation.push(...intents);
			return;
		}
		for (const intent of intents) this.navigateOnce(intent);
	}

	private flushNavigation(): void {
		for (const intent of this.pendingNavigation.splice(0))
			this.navigateOnce(intent);
	}

	private navigateOnce(intent: NavigateIntent): void {
		try {
			this.navigateTo(intent);
		} catch (error) {
			console.warn("[form-workbench] navigation failed", intent, error);
		}
	}

	// ─── Timers and the end ─────────────────────────────────────────────────

	private later(ms: number, callback: () => void): void {
		const handle = this.timers.setTimeout(() => {
			this.scheduled.delete(handle);
			callback();
		}, ms);
		this.scheduled.add(handle);
	}

	private scheduleRetry(afterMs: number): void {
		this.clearRetryTimer();
		this.retryTimer = this.timers.setTimeout(() => {
			this.retryTimer = null;
			this.report({ type: "retryDue" });
		}, afterMs + RETRY_SLACK_MS);
	}

	private clearRetryTimer(): void {
		if (this.retryTimer === null) return;
		this.timers.clearTimeout(this.retryTimer);
		this.retryTimer = null;
	}

	private clearLeaveTimer(): void {
		if (this.leaveTimer === null) return;
		this.timers.clearTimeout(this.leaveTimer);
		this.leaveTimer = null;
	}

	private leave(): void {
		this.leaveTimer = null;
		if (this.attachCount > 0 || this.disposed) return;
		this.left = true;
		this.pendingNavigation = [];
		this.clearRetryTimer();
		this.report({ type: "detached" });
		this.releaseStreams();
		this.disposeWhenIdle();
	}

	/** A left form ends once its last run settled; nothing can attach it later. */
	private disposeWhenIdle(): void {
		if (this.disposed || !this.left || this.attachCount > 0) return;
		if (this.runs.size > 0) return;
		if (this.draining) {
			this.disposePending = true;
			return;
		}
		this.dispose();
	}

	private dispose(): void {
		this.disposed = true;
		for (const upload of this.uploads.values()) upload.abort();
		this.uploads.clear();
		this.files.clear();
		this.clearLeaveTimer();
		this.clearRetryTimer();
		for (const handle of this.scheduled) this.timers.clearTimeout(handle);
		this.scheduled.clear();
		for (const listener of [...this.disposeListeners]) listener();
		this.disposeListeners.clear();
	}
}

export const createSessionController = (options: SessionControllerOptions) =>
	new FormSessionController(options);
