/*
 * A small driver over `reduceSession` for tests (S-STATE's and S-RUNTIME's): it keeps the state and
 * a clock, records every effect, and plays the runtime's part where a test needs it (uploads that
 * finish, runs that are accepted, send output and settle). Not used by the product.
 */
import {
	type FileRef,
	type FormSessionState,
	type PickedFile,
	REQUEST_FILES_STORE_REF,
	type RunOutcome,
	type RunOutput,
	type SessionClock,
	type SessionCommand,
	type SessionEffect,
	type SessionInput,
	type SessionMessage,
	type TerminalSignals,
} from "../contracts";
import { slotsOf } from "../model/values";
import { reduceSession } from "./reduce";

const NO_SIGNALS: TerminalSignals = {
	runInitiated: false,
	errorMessage: null,
	completedStatus: null,
	rejectedStage: null,
	logLevel: null,
	durationMs: null,
};

type UploadEffect = Extract<SessionEffect, { readonly type: "upload" }>;

export interface SessionDriver {
	readonly state: FormSessionState;
	readonly now: number;
	readonly today: string;
	/** Effects of the last message. */
	readonly last: readonly SessionEffect[];
	/** Every effect so far, in order. */
	readonly effects: readonly SessionEffect[];
	/** The effects of each message, one list per message. */
	readonly steps: readonly (readonly SessionEffect[])[];
	/** Uploads asked for and not answered yet, in the order they were asked for. */
	readonly openUploads: readonly UploadEffect[];
	command(command: SessionCommand): SessionDriver;
	input(input: SessionInput): SessionDriver;
	advance(ms: number): SessionDriver;
	/** Answers open uploads (also those they start) until none is left. */
	sendUploads(): SessionDriver;
	/** Answers one open upload. */
	sendUpload(slotId: string): SessionDriver;
	failUpload(slotId: string, error?: string): SessionDriver;
	/** `runAccepted` for every dispatched run not accepted yet. */
	acceptRuns(): SessionDriver;
	/** Output (optional) and the settlement of a run. */
	settle(runId: string, outcome: RunOutcome, output?: RunOutput): SessionDriver;
}

/** What an upload gives back: a FlowPath for FlowPath fields, a URL for legacy ones. */
export function refOf(effect: UploadEffect): FileRef {
	if (effect.mode === "url")
		return { kind: "url", url: `https://files.test/${effect.slotId}` };
	return {
		kind: "flowpath",
		flowPath: {
			path: `tmp/global/requests/${effect.slotId}`,
			store_ref: REQUEST_FILES_STORE_REF,
			cache_store_ref: null,
		},
		url: null,
	};
}

/** A run's output with an answer (or none), as an accumulator would report it. */
export function outputWith(answer = "", eventCount = 1): RunOutput {
	return {
		eventCount,
		steps: [],
		answer,
		reasoning: null,
		attachments: [],
		result: null,
		interactions: [],
		terminal: NO_SIGNALS,
	};
}

/** The FieldKey the state asks the shell to focus, or null. */
export function focusedKey(state: FormSessionState): string | null {
	const target = state.view.focus?.target;
	return target?.kind === "field" ? target.key : null;
}

/** File names a value holds, joined ("" for none). */
export const fileNames = (value: unknown) =>
	slotsOf(value)
		.map((slot) => slot.name)
		.join(", ");

/** Picked files with ids `<prefix>-<name>`. */
export function pickedFiles(
	files: readonly { readonly name: string; readonly size: number }[],
	prefix = "pick",
): readonly PickedFile[] {
	return files.map((file) => ({
		slotId: `${prefix}-${file.name}`,
		name: file.name,
		size: file.size,
		type: "application/pdf",
	}));
}

export function sessionDriver(
	initial: FormSessionState,
	clock: SessionClock,
): SessionDriver {
	let state = initial;
	let now = clock.now;
	let last: readonly SessionEffect[] = [];
	const effects: SessionEffect[] = [];
	const steps: (readonly SessionEffect[])[] = [];
	const uploads = new Map<string, UploadEffect>();
	const dispatched: string[] = [];
	const accepted = new Set<string>();

	const track = (effect: SessionEffect) => {
		if (effect.type === "upload") uploads.set(effect.slotId, effect);
		if (effect.type === "abortUpload") uploads.delete(effect.slotId);
		if (effect.type === "dispatchRun") dispatched.push(effect.runId);
	};

	const step = (message: SessionMessage) => {
		const result = reduceSession(state, message, { now, today: clock.today });
		state = result.state;
		last = result.effects;
		steps.push(result.effects);
		effects.push(...result.effects);
		result.effects.forEach(track);
		return api;
	};

	const sendUpload = (slotId: string) => {
		const effect = uploads.get(slotId);
		if (!effect) return api;
		uploads.delete(slotId);
		return step({
			kind: "input",
			input: {
				type: "uploadSent",
				slotId,
				ref: refOf(effect),
				expiresAt: null,
			},
		});
	};

	const api: SessionDriver = {
		get state() {
			return state;
		},
		get now() {
			return now;
		},
		today: clock.today,
		get last() {
			return last;
		},
		effects,
		steps,
		get openUploads() {
			return [...uploads.values()];
		},
		command: (command) => step({ kind: "command", command }),
		input: (input) => step({ kind: "input", input }),
		advance: (ms) => {
			now += ms;
			return api;
		},
		sendUpload,
		sendUploads: () => {
			for (let first = uploads.keys().next(); !first.done; ) {
				sendUpload(first.value);
				first = uploads.keys().next();
			}
			return api;
		},
		failUpload: (slotId, error = "The upload failed.") => {
			uploads.delete(slotId);
			return step({
				kind: "input",
				input: { type: "uploadFailed", slotId, error },
			});
		},
		acceptRuns: () => {
			for (const runId of dispatched) {
				if (accepted.has(runId)) continue;
				accepted.add(runId);
				step({
					kind: "input",
					input: {
						type: "runAccepted",
						runId,
						backendRunId: `backend-${runId}`,
					},
				});
			}
			return api;
		},
		settle: (runId, outcome, output) => {
			if (output)
				step({ kind: "input", input: { type: "runOutput", runId, output } });
			return step({
				kind: "input",
				input: { type: "runSettled", runId, outcome },
			});
		},
	};
	return api;
}
