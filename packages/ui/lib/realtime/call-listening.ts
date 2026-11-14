import { isHostedAudioProvider } from "../bit/hosted-audio";
import { type IBit, IBitTypes } from "../schema";

export const STT_LOCAL_PROVIDER = "local:any-speech-to-text";
export const LISTEN_SAMPLE_RATE = 16_000;
export const MIN_UTTERANCE_MS = 600;
export const END_SILENCE_MS = 450;
export const MAX_UTTERANCE_MS = 15_000;
export const MAX_QUEUED_UTTERANCES = 3;

const TRAILING_KEEP_MS = 150;
const PRE_ROLL_MS = 200;
const DEFAULT_THRESHOLD = 0.015;
export const BARGE_IN_THRESHOLD = DEFAULT_THRESHOLD * 3;
export const BARGE_IN_MIN_LOUD_MS = 300;
export const BARGE_IN_WINDOW_MS = 800;
export const BARGE_IN_CONFIRM_MS = 500;
export const BARGE_IN_COOLDOWN_MS = 2000;
export const BARGE_IN_VAD_PROBABILITY = 0.6;
export const BARGE_IN_VAD_MIN_MS = 250;
export const BARGE_IN_VAD_WINDOW_MS = 500;

export const VAD_START_PROBABILITY = 0.5;
export const VAD_CONTINUE_PROBABILITY = 0.35;
export const MIN_SPEECH_MS = 250;
export const TURN_CHECK_SILENCE_MS = 200;
export const TURN_COMPLETE_PROBABILITY = 0.5;
export const INCOMPLETE_TURN_SILENCE_MS = 1500;

export type TurnSource = "model" | "silence" | "max" | "flush";

export interface TurnDecision {
	source: TurnSource;
	probability?: number;
	/** From the end of speech to the decision. */
	waitedMs: number;
}

export interface Utterance {
	samples: Float32Array;
	/** Where the utterance starts, in ms since the segmenter began. */
	offsetMs: number;
	durationMs: number;
	/** Where the last speech ends, in ms since the segmenter began. */
	speechEndMs?: number;
	/** Wall-clock start of the captured audio, when supplied by the microphone listener. */
	capturedAt?: number;
	/** Wall clock of the last speech sample, when supplied by the microphone listener. */
	endOfSpeechAt?: number;
	/** A preview whose samples are exactly what the final utterance returns if no more speech follows. */
	settled?: boolean;
	turn?: TurnDecision;
}

export interface SegmenterOptions {
	sampleRate?: number;
	threshold?: number;
	minMs?: number;
	silenceMs?: number;
	maxMs?: number;
	/** Cumulative audio snapshots while speech is still being captured. */
	onPartial?: (utterance: Utterance) => void;
	partialMs?: number;
	/** Speech by voice-activity probability only needs this much speech. */
	minSpeechMs?: number;
	turnCheckMs?: number;
	incompleteSilenceMs?: number;
	/**
	 * Asks whether the speaker finished; answer later with `answerTurn(id, probability)`, never
	 * synchronously. Returning false means no turn model is available.
	 */
	onTurnCheck?: (id: number, samples: Float32Array) => boolean;
}

export function rms(samples: ArrayLike<number>): number {
	if (samples.length === 0) return 0;
	let sum = 0;
	for (let i = 0; i < samples.length; i++) sum += samples[i] * samples[i];
	return Math.sqrt(sum / samples.length);
}

function concat(parts: readonly Float32Array[], length: number): Float32Array {
	const out = new Float32Array(length);
	let at = 0;
	for (const part of parts) {
		out.set(part, at);
		at += part.length;
	}
	return out;
}

/**
 * Cuts a stream of mono PCM chunks into utterances by energy: an utterance starts at the first
 * loud chunk, ends after `silenceMs` of quiet, is dropped under `minMs` of speech and is cut at `maxMs`.
 */
export function createUtteranceSegmenter(options: SegmenterOptions = {}) {
	const rate = options.sampleRate ?? LISTEN_SAMPLE_RATE;
	const threshold = options.threshold ?? DEFAULT_THRESHOLD;
	const perMs = rate / 1000;
	const minSamples = (options.minMs ?? MIN_UTTERANCE_MS) * perMs;
	const silenceSamples = (options.silenceMs ?? END_SILENCE_MS) * perMs;
	const maxSamples = (options.maxMs ?? MAX_UTTERANCE_MS) * perMs;
	const keepSamples = TRAILING_KEEP_MS * perMs;
	const preRollSamples = PRE_ROLL_MS * perMs;
	const partialSamples = (options.partialMs ?? 1000) * perMs;
	const minSpeechSamples = (options.minSpeechMs ?? MIN_SPEECH_MS) * perMs;
	const turnCheckSamples =
		(options.turnCheckMs ?? TURN_CHECK_SILENCE_MS) * perMs;
	const incompleteSamples =
		(options.incompleteSilenceMs ?? INCOMPLETE_TURN_SILENCE_MS) * perMs;

	let position = 0;
	let preRoll: Float32Array[] = [];
	let preRollLength = 0;
	let parts: Float32Array[] = [];
	let length = 0;
	let start = 0;
	let lastLoudEnd = 0;
	let speaking = false;
	let lastPartialLength = 0;
	let settledAt = -1;
	let modelMode = false;
	let modelVoiced = false;
	let wasVoiced = false;
	let speechSamples = 0;
	let turnSequence = 0;
	let pendingTurn = 0;
	let verdict: "none" | "incomplete" | "unavailable" = "none";
	let incompleteProbability: number | undefined;

	// Once the model heard speech in this utterance its short minimum holds, even for chunks whose
	// answer came too late and were judged by energy.
	const enoughSpeech = () =>
		modelVoiced
			? speechSamples >= minSpeechSamples
			: lastLoudEnd - start >= minSamples;
	const onsetSamples = () => (modelMode ? turnCheckSamples : keepSamples);
	// Only an answer in hand extends the wait: a slow turn model never ends a turn later than the
	// plain silence rule would.
	const silenceLimit = () =>
		verdict === "incomplete" ? incompleteSamples : silenceSamples;

	const isVoiced = (chunk: Float32Array, probability?: number) =>
		probability === undefined
			? rms(chunk) >= threshold
			: probability >=
				(wasVoiced ? VAD_CONTINUE_PROBABILITY : VAD_START_PROBABILITY);

	const settle = () => {
		if (!options.onPartial || settledAt === lastLoudEnd) return;
		if (
			!enoughSpeech() ||
			position - lastLoudEnd < onsetSamples() ||
			length >= maxSamples
		)
			return;
		settledAt = lastLoudEnd;
		const end = Math.min(length, lastLoudEnd - start + keepSamples);
		options.onPartial({
			samples: concat(parts, length).subarray(0, end).slice(),
			offsetMs: Math.round(start / perMs),
			durationMs: Math.round(end / perMs),
			settled: true,
		});
	};

	const preview = () => {
		if (
			!options.onPartial ||
			!enoughSpeech() ||
			length - lastPartialLength < partialSamples
		)
			return;
		lastPartialLength = length;
		options.onPartial({
			samples: concat(parts, length),
			offsetMs: Math.round(start / perMs),
			durationMs: Math.round(length / perMs),
		});
	};

	const clearTurn = () => {
		pendingTurn = 0;
		verdict = "none";
		incompleteProbability = undefined;
	};

	const finish = (source: TurnSource, probability?: number): Utterance[] => {
		const enough = enoughSpeech();
		const end = Math.min(length, lastLoudEnd - start + keepSamples);
		const samples = concat(parts, length).subarray(0, end);
		const startAt = start;
		const turn: TurnDecision = {
			source,
			waitedMs: Math.round((position - lastLoudEnd) / perMs),
		};
		if (probability !== undefined) turn.probability = probability;
		const speechEndMs = Math.round(lastLoudEnd / perMs);
		speaking = false;
		parts = [];
		length = 0;
		lastPartialLength = 0;
		settledAt = -1;
		speechSamples = 0;
		modelVoiced = false;
		clearTurn();
		if (!enough) return [];
		return [
			{
				samples: samples.slice(),
				offsetMs: Math.round(startAt / perMs),
				durationMs: Math.round(samples.length / perMs),
				speechEndMs,
				turn,
			},
		];
	};

	const keepPreRoll = (chunk: Float32Array) => {
		preRoll.push(chunk);
		preRollLength += chunk.length;
		while (preRoll.length > 1 && preRollLength > preRollSamples) {
			preRollLength -= (preRoll.shift() as Float32Array).length;
		}
		position += chunk.length;
	};

	const begin = (chunk: Float32Array) => {
		parts = [...preRoll, chunk];
		length = preRollLength + chunk.length;
		start = position - preRollLength;
		preRoll = [];
		preRollLength = 0;
		speaking = true;
		position += chunk.length;
		lastLoudEnd = position;
		speechSamples = chunk.length;
		modelVoiced = modelMode;
		clearTurn();
		preview();
	};

	const askTurn = () => {
		if (
			!modelMode ||
			!options.onTurnCheck ||
			pendingTurn ||
			verdict !== "none" ||
			position - lastLoudEnd < turnCheckSamples
		)
			return;
		pendingTurn = ++turnSequence;
		if (!options.onTurnCheck(pendingTurn, concat(parts, length))) {
			pendingTurn = 0;
			verdict = "unavailable";
		}
	};

	return {
		/**
		 * `probability` is the voice-activity model's answer for this chunk; without it the chunk
		 * is judged by energy and the energy rules apply.
		 */
		push(chunk: Float32Array, probability?: number): Utterance[] {
			modelMode = probability !== undefined;
			const loud = isVoiced(chunk, probability);
			wasVoiced = loud;
			if (!speaking) {
				if (loud) begin(chunk);
				else keepPreRoll(chunk);
				return [];
			}
			parts.push(chunk);
			length += chunk.length;
			position += chunk.length;
			if (loud) {
				lastLoudEnd = position;
				speechSamples += chunk.length;
				modelVoiced ||= modelMode;
				clearTurn();
			}
			if (length >= maxSamples) return finish("max");
			if (position - lastLoudEnd >= silenceLimit()) {
				return finish("silence", incompleteProbability);
			}
			if (loud) preview();
			else {
				settle();
				askTurn();
			}
			return [];
		},
		/** The turn model's verdict for a check asked by `onTurnCheck`; null when it could not answer. */
		answerTurn(id: number, probability: number | null): Utterance[] {
			if (!speaking || id !== pendingTurn) return [];
			pendingTurn = 0;
			if (probability === null) {
				verdict = "unavailable";
				return [];
			}
			if (probability >= TURN_COMPLETE_PROBABILITY) {
				return finish("model", probability);
			}
			verdict = "incomplete";
			incompleteProbability = probability;
			return [];
		},
		flush(): Utterance | null {
			return speaking ? (finish("flush")[0] ?? null) : null;
		},
		reset() {
			speaking = false;
			wasVoiced = false;
			modelVoiced = false;
			speechSamples = 0;
			parts = [];
			length = 0;
			preRoll = [];
			preRollLength = 0;
			lastPartialLength = 0;
			settledAt = -1;
			clearTurn();
		},
	};
}

/**
 * Detects a possible interruption from sustained speech within a short window, allowing gaps
 * between syllables. Without a voice-activity model it requires loud audio instead. Returns
 * audio starting just before the onset for the listener's separate transcript and echo check.
 */
export function createBargeInDetector(
	options: {
		sampleRate?: number;
		threshold?: number;
		minLoudMs?: number;
		windowMs?: number;
		vadMinMs?: number;
		vadWindowMs?: number;
		vadProbability?: number;
	} = {},
) {
	const rate = options.sampleRate ?? LISTEN_SAMPLE_RATE;
	const threshold = options.threshold ?? BARGE_IN_THRESHOLD;
	const perMs = rate / 1000;
	const vadProbability = options.vadProbability ?? BARGE_IN_VAD_PROBABILITY;
	const limits = {
		energy: {
			minLoud: (options.minLoudMs ?? BARGE_IN_MIN_LOUD_MS) * perMs,
			window: (options.windowMs ?? BARGE_IN_WINDOW_MS) * perMs,
		},
		vad: {
			minLoud: (options.vadMinMs ?? BARGE_IN_VAD_MIN_MS) * perMs,
			window: (options.vadWindowMs ?? BARGE_IN_VAD_WINDOW_MS) * perMs,
		},
	};
	let entries: { chunk: Float32Array; loud: boolean }[] = [];
	let total = 0;
	let loudTotal = 0;

	const reset = () => {
		entries = [];
		total = 0;
		loudTotal = 0;
	};

	const trim = (windowSamples: number) => {
		while (entries.length > 1 && total > windowSamples) {
			const dropped = entries.shift();
			if (!dropped) break;
			total -= dropped.chunk.length;
			if (dropped.loud) loudTotal -= dropped.chunk.length;
		}
	};

	return {
		push(chunk: Float32Array, probability?: number): Float32Array[] | null {
			const { minLoud, window: windowSamples } =
				probability === undefined ? limits.energy : limits.vad;
			// Echo cancellation and microphone gain can attenuate real speech below the energy
			// fallback's threshold. A model answer must not also pass that absolute volume gate.
			const loud =
				probability === undefined
					? rms(chunk) >= threshold
					: probability >= vadProbability;
			entries.push({ chunk, loud });
			total += chunk.length;
			if (loud) loudTotal += chunk.length;
			trim(windowSamples);
			if (loudTotal < minLoud) return null;
			const first = entries.findIndex((entry) => entry.loud);
			const found = entries.slice(Math.max(0, first - 1)).map((e) => e.chunk);
			reset();
			return found;
		},
		reset,
	};
}

/** Averages `input` down (or repeats it up) to `toRate`; mono in, mono out. */
export function resample(
	input: Float32Array,
	fromRate: number,
	toRate: number = LISTEN_SAMPLE_RATE,
): Float32Array {
	if (fromRate === toRate || input.length === 0) return input;
	const ratio = fromRate / toRate;
	const out = new Float32Array(Math.floor(input.length / ratio));
	for (let i = 0; i < out.length; i++) {
		const from = Math.floor(i * ratio);
		const to = Math.max(
			from + 1,
			Math.min(input.length, Math.floor((i + 1) * ratio)),
		);
		let sum = 0;
		for (let j = from; j < to; j++) sum += input[j];
		out[i] = sum / (to - from);
	}
	return out;
}

/** A 16-bit PCM mono WAV file. */
export function encodeWav(
	samples: Float32Array,
	sampleRate: number = LISTEN_SAMPLE_RATE,
): Uint8Array<ArrayBuffer> {
	const dataBytes = samples.length * 2;
	const buffer = new ArrayBuffer(44 + dataBytes);
	const view = new DataView(buffer);
	const tag = (at: number, text: string) => {
		for (let i = 0; i < text.length; i++)
			view.setUint8(at + i, text.charCodeAt(i));
	};
	tag(0, "RIFF");
	view.setUint32(4, 36 + dataBytes, true);
	tag(8, "WAVE");
	tag(12, "fmt ");
	view.setUint32(16, 16, true);
	view.setUint16(20, 1, true);
	view.setUint16(22, 1, true);
	view.setUint32(24, sampleRate, true);
	view.setUint32(28, sampleRate * 2, true);
	view.setUint16(32, 2, true);
	view.setUint16(34, 16, true);
	tag(36, "data");
	view.setUint32(40, dataBytes, true);
	for (let i = 0; i < samples.length; i++) {
		const v = Math.max(-1, Math.min(1, samples[i]));
		view.setInt16(44 + i * 2, v < 0 ? v * 0x8000 : v * 0x7fff, true);
	}
	return new Uint8Array(buffer);
}

export function bytesToBase64(bytes: Uint8Array): string {
	let binary = "";
	const step = 0x8000;
	for (let i = 0; i < bytes.length; i += step) {
		binary += String.fromCharCode(...bytes.subarray(i, i + step));
	}
	return btoa(binary);
}

export type SttRoute = "local" | "remote";

export interface SttChoice {
	/** The id the transcribe request carries; local bits are addressed with their hub. */
	bitId: string;
	route: SttRoute;
}

function providerName(bit: IBit): string {
	const name = bit.parameters?.provider?.provider_name;
	return typeof name === "string" ? name.trim().toLowerCase() : "";
}

export function isLocalSttBit(bit: IBit): boolean {
	return bit.type === IBitTypes.Stt && providerName(bit) === STT_LOCAL_PROVIDER;
}

/**
 * Which speech-to-text bit of the profile transcribes the speaker: on desktop an installed local
 * model first, then a model supported by the hosted audio gateway; null when the profile has neither.
 * `localOnly` leaves the hosted gateway out: it would hear what an end-to-end encrypted
 * conversation keeps from the server.
 */
export function resolveSttChoice(
	bits: readonly IBit[],
	options: {
		desktop: boolean;
		installed: ReadonlySet<string>;
		localOnly?: boolean;
	},
): SttChoice | null {
	const stt = bits.filter((bit) => bit.type === IBitTypes.Stt);
	if (options.desktop) {
		const local = stt.find(
			(bit) => isLocalSttBit(bit) && options.installed.has(bit.id),
		);
		if (local) {
			return {
				bitId: local.hub ? `${local.hub}:${local.id}` : local.id,
				route: "local",
			};
		}
	}
	if (options.localOnly) return null;
	const remote = stt.find((bit) => isHostedAudioProvider(providerName(bit)));
	return remote ? { bitId: remote.id, route: "remote" } : null;
}

/**
 * Whether this device offers calls in a conversation. An end-to-end encrypted one does only
 * with a speech model on the device: hosted speech runs behind the server, which must not hear
 * what it can't read.
 */
export function offersCalls(
	encrypted: boolean,
	speech: SttChoice | null,
): boolean {
	return !encrypted || speech?.route === "local";
}

/** One request at a time; once more than `limit` wait, the oldest are dropped. */
export function createSerialQueue<T>(
	run: (item: T) => Promise<void>,
	limit: number = MAX_QUEUED_UTTERANCES,
) {
	const waiting: T[] = [];
	let busy = false;
	let closed = false;

	const next = async () => {
		if (busy || closed) return;
		const item = waiting.shift();
		if (item === undefined) return;
		busy = true;
		try {
			await run(item);
		} catch (cause) {
			console.warn("Transcribing an utterance failed", cause);
		} finally {
			busy = false;
			void next();
		}
	};

	return {
		push(item: T) {
			if (closed) return;
			waiting.push(item);
			while (waiting.length > limit) waiting.shift();
			void next();
		},
		get size() {
			return waiting.length + (busy ? 1 : 0);
		},
		close() {
			closed = true;
			waiting.length = 0;
		},
	};
}
