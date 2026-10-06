/*
 * The session reducer (PLAN §3.2): `reduceSession(state, message, clock) → { state, effects }`, pure
 * and deterministic. Each message goes to its area's handler; then every step ends the same way: a
 * message past its time goes, queued runs start while places are free, uploads nothing needs are
 * aborted and Files the last holder dropped are released, waiting uploads start (two at a time), a
 * start message follows its run's state (a failed answer's message goes with its run), and no
 * effect is asked for twice.
 */
import type {
	FormSessionState,
	LoadedMemory,
	ReduceSession,
	SessionClock,
	SessionCommand,
	SessionEffect,
	SessionInput,
	SessionStep,
} from "../contracts";
import { FILE_COMMANDS, FILE_INPUTS } from "./reduce-files";
import { LIFECYCLE_INPUTS } from "./reduce-lifecycle";
import { MEMORY_COMMANDS, MEMORY_INPUTS } from "./reduce-memory";
import { PER_RUN_COMMANDS } from "./reduce-per-run";
import { PRESET_COMMANDS } from "./reduce-presets";
import { PRESS_COMMANDS, withLiveStart } from "./reduce-press";
import { QUEUE_COMMANDS, QUEUE_INPUTS } from "./reduce-queue";
import { RAIL_COMMANDS } from "./reduce-rail";
import { startQueued } from "./reduce-runs";
import {
	type CommandHandlers,
	type InputHandlers,
	type Tx,
	createTx,
	withView,
} from "./reduce-tx";
import {
	VIEW_COMMANDS,
	VIEW_INPUTS,
	withoutStaleAnswerFailure,
} from "./reduce-view";
import { scheduleUploads, settleSlots } from "./slots";

const COMMANDS: CommandHandlers = {
	...RAIL_COMMANDS,
	...FILE_COMMANDS,
	...PRESS_COMMANDS,
	...QUEUE_COMMANDS,
	...PER_RUN_COMMANDS,
	...MEMORY_COMMANDS,
	...PRESET_COMMANDS,
	...VIEW_COMMANDS,
};

const INPUTS: InputHandlers = {
	...MEMORY_INPUTS,
	...FILE_INPUTS,
	...QUEUE_INPUTS,
	...LIFECYCLE_INPUTS,
	...VIEW_INPUTS,
};

type Handler<M> = (
	state: FormSessionState,
	message: M,
	tx: Tx,
) => FormSessionState;

function runCommand(state: FormSessionState, command: SessionCommand, tx: Tx) {
	const handler = COMMANDS[command.type] as Handler<SessionCommand>;
	return handler(state, command, tx);
}

function runInput(state: FormSessionState, input: SessionInput, tx: Tx) {
	const handler = INPUTS[input.type] as Handler<SessionInput>;
	return handler(state, input, tx);
}

/** A plain message whose time is up goes even when its timer's input has not arrived yet. */
function withoutStaleMessage(state: FormSessionState, now: number) {
	const expiresAt = state.view.message?.expiresAt ?? null;
	return expiresAt !== null && expiresAt <= now
		? withView(state, { message: null })
		: state;
}

type EffectKey = {
	readonly [T in SessionEffect["type"]]: (
		effect: Extract<SessionEffect, { readonly type: T }>,
	) => string | null;
};

/** Effects with the same key collapse to the last one asked for (it carries the newest data). */
const EFFECT_KEY: EffectKey = {
	upload: (effect) => `upload:${effect.slotId}`,
	abortUpload: (effect) => `abort:${effect.slotId}`,
	releaseFiles: () => "release",
	dispatchRun: (effect) => `dispatch:${effect.runId}`,
	stopRun: (effect) => `stop:${effect.runId}`,
	persistRun: (effect) => `run:${effect.record.id}`,
	deleteRun: (effect) => `delete-run:${effect.id}`,
	hideField: (effect) => `hide:${effect.name}`,
	persistPrefs: () => "prefs",
	persistPreset: (effect) => `preset:${effect.preset.id}`,
	deletePreset: (effect) => `delete-preset:${effect.presetId}`,
	expireMessage: (effect) => `expire:${effect.seq}`,
	scheduleRetry: () => "retry",
	respondInteraction: () => null,
};

const keyOf = (effect: SessionEffect) =>
	(EFFECT_KEY[effect.type] as (effect: SessionEffect) => string | null)(effect);

/** One effect per key, at the place of its last request. */
export function distinctEffects(
	effects: readonly SessionEffect[],
): readonly SessionEffect[] {
	const last = new Map<string, number>();
	effects.forEach((effect, index) => {
		const key = keyOf(effect);
		if (key !== null) last.set(key, index);
	});
	return effects.filter((effect, index) => {
		const key = keyOf(effect);
		return key === null || last.get(key) === index;
	});
}

function finish(
	before: FormSessionState,
	after: FormSessionState,
	tx: Tx,
): SessionStep {
	let state = withoutStaleMessage(after, tx.clock.now);
	state = startQueued(state, tx);
	state = settleSlots(before, state, tx);
	state = scheduleUploads(state, tx);
	state = withoutStaleAnswerFailure(withLiveStart(state));
	if (state.seq !== tx.seq) state = { ...state, seq: tx.seq };
	return { state, effects: distinctEffects(tx.effects) };
}

export const reduceSession: ReduceSession = (state, message, clock) => {
	const tx = createTx(state, clock);
	const next =
		message.kind === "command"
			? runCommand(state, message.command, tx)
			: runInput(state, message.input, tx);
	return finish(state, next, tx);
};

/** `memoryLoaded` as one step: saved runs, prefs and presets merged into the session. */
export const applyLoadedMemory = (
	state: FormSessionState,
	memory: LoadedMemory,
	clock: SessionClock,
): SessionStep =>
	reduceSession(
		state,
		{ kind: "input", input: { type: "memoryLoaded", memory } },
		clock,
	);
