import type { ISpeechStream } from "../../state/backend-state/audio-state";
import { createCallCues } from "./call-cues";

export const DUCKED_GAIN = 0.2;
export const STREAM_START_BUFFER_SECONDS = 0.12;
const DUCK_RAMP_SECONDS = 0.03;

export interface SpeechProgress {
	elapsedMs: number;
	durationMs: number;
}

interface Timeline {
	botId: string;
	buffers: AudioBuffer[];
	starts: number[];
	total: number;
	sources: Set<AudioBufferSourceNode>;
	origin: number;
	next: number;
	started: boolean;
	paused: boolean;
	pausedAt: number;
	ended: boolean;
	closed: boolean;
	release?: () => void;
	resolve: () => void;
	reject: (cause: unknown) => void;
}

/** The whole 16-bit samples of `carry` followed by `chunk`, and the odd byte left for the next chunk. */
function wholeSamples(carry: Uint8Array | null, chunk: Uint8Array) {
	let bytes = chunk;
	if (carry) {
		bytes = new Uint8Array(carry.length + chunk.length);
		bytes.set(carry);
		bytes.set(chunk, carry.length);
	}
	const usable = bytes.length - (bytes.length % 2);
	return {
		samples: bytes.subarray(0, usable),
		rest: usable < bytes.length ? bytes.slice(usable) : null,
	};
}

/** Mixes generated speech into the outgoing microphone stream and plays it locally. */
export function createCallSpeechOutput(
	context: AudioContext,
	microphone: MediaStream,
	onSpeaking: (botId: string | null) => void,
) {
	const destination = context.createMediaStreamDestination();
	const input = context.createMediaStreamSource(microphone);
	input.connect(destination);
	const level = context.createGain();
	level.connect(destination);
	level.connect(context.destination);
	const cue = createCallCues(context, context.destination);
	let timeline: Timeline | null = null;
	let generation = 0;
	let disposed = false;
	let finish: (() => void) | null = null;

	const clearSources = (line: Timeline) => {
		for (const source of line.sources) {
			source.onended = null;
			try {
				source.stop();
			} catch {}
			source.disconnect();
		}
		line.sources.clear();
	};

	const closeLine = (line: Timeline, failure?: { error: unknown }) => {
		if (line.closed) return;
		line.closed = true;
		clearSources(line);
		line.release?.();
		if (timeline === line) {
			timeline = null;
			finish = null;
		}
		if (line.started) onSpeaking(null);
		if (failure) line.reject(failure.error);
		else line.resolve();
	};

	const check = (line: Timeline) => {
		if (
			line.ended &&
			!line.paused &&
			line.sources.size === 0 &&
			line.next >= line.buffers.length
		)
			closeLine(line);
	};

	const position = (line: Timeline) =>
		line.paused
			? line.pausedAt
			: Math.min(line.total, Math.max(0, context.currentTime - line.origin));

	const scheduleBuffer = (line: Timeline, index: number, at: number) => {
		const buffer = line.buffers[index];
		const source = context.createBufferSource();
		source.buffer = buffer;
		source.connect(level);
		line.sources.add(source);
		source.onended = () => {
			if (!line.sources.delete(source)) return;
			source.onended = null;
			source.disconnect();
			check(line);
		};
		const start = line.starts[index];
		source.start(
			Math.max(context.currentTime, line.origin + start),
			Math.max(0, at - start),
		);
	};

	const scheduleFrom = (line: Timeline, at: number) => {
		line.origin = context.currentTime - at;
		for (let index = 0; index < line.buffers.length; index++) {
			if (at > 0 && line.starts[index] + line.buffers[index].duration <= at)
				continue;
			scheduleBuffer(line, index, at);
		}
		line.next = line.buffers.length;
	};

	const startLine = (line: Timeline) => {
		line.started = true;
		onSpeaking(line.botId);
		scheduleFrom(line, 0);
	};

	const openLine = (botId: string) => {
		const line = { botId } as Timeline;
		const promise = new Promise<void>((resolve, reject) => {
			Object.assign(line, {
				buffers: [],
				starts: [],
				total: 0,
				sources: new Set(),
				origin: 0,
				next: 0,
				started: false,
				paused: false,
				pausedAt: 0,
				ended: false,
				closed: false,
				resolve,
				reject,
			});
		});
		timeline = line;
		finish = () => closeLine(line);
		return { line, promise };
	};

	const append = (line: Timeline, buffer: AudioBuffer) => {
		const index = line.buffers.length;
		line.starts.push(line.total);
		line.buffers.push(buffer);
		line.total += buffer.duration;
		if (!line.started) {
			if (line.total >= STREAM_START_BUFFER_SECONDS) startLine(line);
			return;
		}
		if (line.paused) return;
		if (line.origin + line.starts[index] < context.currentTime)
			line.origin = context.currentTime - line.starts[index];
		scheduleBuffer(line, index, 0);
		line.next = index + 1;
	};

	const pcmBuffer = (bytes: Uint8Array, sampleRate: number) => {
		const view = new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
		const samples = bytes.byteLength / 2;
		const buffer = context.createBuffer(1, samples, sampleRate);
		const channel = buffer.getChannelData(0);
		for (let index = 0; index < samples; index++)
			channel[index] = view.getInt16(index * 2, true) / 32768;
		return buffer;
	};

	const readStream = async (
		line: Timeline,
		iterator: AsyncIterator<Uint8Array>,
		sampleRate: number,
	) => {
		let carry: Uint8Array | null = null;
		try {
			for (;;) {
				const result = await iterator.next();
				if (line.closed) return;
				if (result.done) break;
				const { samples, rest } = wholeSamples(carry, result.value);
				carry = rest;
				if (samples.length > 0) append(line, pcmBuffer(samples, sampleRate));
			}
			line.ended = true;
			if (!line.started && line.buffers.length > 0) startLine(line);
			check(line);
		} catch (error) {
			closeLine(line, { error });
		}
	};

	const stop = () => {
		generation++;
		finish?.();
	};

	const ready = async (current: number) => {
		await context.resume();
		if (disposed || current !== generation) return false;
		if (context.state !== "running")
			throw new Error("Call audio playback is unavailable");
		return true;
	};

	return {
		stream: destination.stream,
		cue,
		async play(blob: Blob, botId: string): Promise<void> {
			stop();
			if (disposed) return;
			const current = generation;
			const buffer = await context.decodeAudioData(await blob.arrayBuffer());
			if (disposed || current !== generation) return;
			if (!(await ready(current))) return;
			const { line, promise } = openLine(botId);
			line.buffers.push(buffer);
			line.starts.push(0);
			line.total = buffer.duration;
			line.ended = true;
			try {
				startLine(line);
			} catch (error) {
				// Reject before cleanup resolves the promise so callers can show the text fallback.
				closeLine(line, { error });
			}
			return promise;
		},
		/** Plays PCM as it arrives, starting once about 120 ms is buffered. */
		async playStream(speech: ISpeechStream, botId: string): Promise<void> {
			stop();
			if (disposed) return;
			const current = generation;
			if (!(await ready(current))) return;
			const iterator = speech.chunks[Symbol.asyncIterator]();
			const { line, promise } = openLine(botId);
			line.release = () => {
				Promise.resolve(iterator.return?.()).catch(() => {});
			};
			void readStream(line, iterator, speech.sampleRate);
			return promise;
		},
		stop,
		/** Holds the speech at its position; false when nothing is playing. */
		pause(): boolean {
			const line = timeline;
			if (!line?.started || line.paused || line.closed || disposed)
				return false;
			line.pausedAt = position(line);
			line.paused = true;
			clearSources(line);
			return true;
		},
		resume() {
			const line = timeline;
			if (!line?.paused || line.closed || disposed) return;
			line.paused = false;
			try {
				scheduleFrom(line, line.pausedAt);
			} catch (error) {
				closeLine(line, { error });
				return;
			}
			check(line);
		},
		/** Lowers the bot's voice on this device and in the outgoing stream, e.g. while checking for a barge-in. */
		duck(on: boolean) {
			if (disposed) return;
			level.gain.setTargetAtTime(
				on ? DUCKED_GAIN : 1,
				context.currentTime,
				DUCK_RAMP_SECONDS,
			);
		},
		/** How far the speech being played has got, or null when nothing is playing. */
		progress(): SpeechProgress | null {
			if (!timeline?.started) return null;
			return {
				elapsedMs: position(timeline) * 1000,
				durationMs: timeline.total * 1000,
			};
		},
		dispose() {
			if (disposed) return;
			disposed = true;
			stop();
			cue.dispose();
			input.disconnect();
			level.disconnect();
			destination.disconnect();
			for (const track of destination.stream.getTracks()) track.stop();
		},
	};
}

export type CallSpeechOutput = ReturnType<typeof createCallSpeechOutput>;
