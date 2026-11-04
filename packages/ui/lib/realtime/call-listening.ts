import { type IBit, IBitTypes } from "../schema";

export const STT_LOCAL_PROVIDER = "local:any-speech-to-text";
export const LISTEN_SAMPLE_RATE = 16_000;
export const MIN_UTTERANCE_MS = 600;
export const END_SILENCE_MS = 700;
export const MAX_UTTERANCE_MS = 15_000;
export const MAX_QUEUED_UTTERANCES = 3;

const TRAILING_KEEP_MS = 150;
const PRE_ROLL_MS = 200;
const DEFAULT_THRESHOLD = 0.015;

export interface Utterance {
	samples: Float32Array;
	/** Where the utterance starts, in ms since the segmenter began. */
	offsetMs: number;
	durationMs: number;
}

export interface SegmenterOptions {
	sampleRate?: number;
	threshold?: number;
	minMs?: number;
	silenceMs?: number;
	maxMs?: number;
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

	let position = 0;
	let preRoll: Float32Array[] = [];
	let preRollLength = 0;
	let parts: Float32Array[] = [];
	let length = 0;
	let start = 0;
	let lastLoudEnd = 0;
	let speaking = false;

	const finish = (): Utterance | null => {
		const speech = lastLoudEnd - start;
		const end = Math.min(length, lastLoudEnd - start + keepSamples);
		const samples = concat(parts, length).subarray(0, end);
		const startAt = start;
		speaking = false;
		parts = [];
		length = 0;
		if (speech < minSamples) return null;
		return {
			samples: samples.slice(),
			offsetMs: Math.round(startAt / perMs),
			durationMs: Math.round(samples.length / perMs),
		};
	};

	return {
		push(chunk: Float32Array): Utterance[] {
			const out: Utterance[] = [];
			const loud = rms(chunk) >= threshold;
			if (!speaking) {
				if (!loud) {
					preRoll.push(chunk);
					preRollLength += chunk.length;
					while (preRoll.length > 1 && preRollLength > preRollSamples) {
						preRollLength -= (preRoll.shift() as Float32Array).length;
					}
					position += chunk.length;
					return out;
				}
				parts = [...preRoll, chunk];
				length = preRollLength + chunk.length;
				start = position - preRollLength;
				preRoll = [];
				preRollLength = 0;
				speaking = true;
				position += chunk.length;
				lastLoudEnd = position;
				return out;
			}
			parts.push(chunk);
			length += chunk.length;
			position += chunk.length;
			if (loud) lastLoudEnd = position;
			if (position - lastLoudEnd >= silenceSamples || length >= maxSamples) {
				const done = finish();
				if (done) out.push(done);
			}
			return out;
		},
		flush(): Utterance | null {
			return speaking ? finish() : null;
		},
		reset() {
			speaking = false;
			parts = [];
			length = 0;
			preRoll = [];
			preRollLength = 0;
		},
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
 * model first, then any hosted or API model; null when the profile has neither.
 */
export function resolveSttChoice(
	bits: readonly IBit[],
	options: { desktop: boolean; installed: ReadonlySet<string> },
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
	const remote = stt.find((bit) => !providerName(bit).startsWith("local:"));
	return remote ? { bitId: remote.id, route: "remote" } : null;
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
