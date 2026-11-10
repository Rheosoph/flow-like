import { describe, expect, test } from "bun:test";
import { createCallSpeechOutput } from "./call-speech-output";

function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (cause: unknown) => void;
	const promise = new Promise<T>((yes, no) => {
		resolve = yes;
		reject = no;
	});
	return { promise, resolve, reject };
}

class FakeNode {
	connections: unknown[] = [];
	disconnects = 0;
	connect(node: unknown) {
		this.connections.push(node);
	}
	disconnect() {
		this.disconnects++;
	}
}

class FakeBufferSource extends FakeNode {
	buffer: AudioBuffer | null = null;
	onended: (() => void) | null = null;
	starts = 0;
	stops = 0;
	startFailure: Error | null = null;
	startArgs: unknown[][] = [];
	start(...args: unknown[]) {
		this.startArgs.push(args);
		this.starts++;
		if (this.startFailure) throw this.startFailure;
	}
	stop() {
		this.stops++;
	}
}

class FakeGain extends FakeNode {
	targets: { value: number; at: number }[] = [];
	gain = {
		setTargetAtTime: (value: number, at: number) => {
			this.targets.push({ value, at });
		},
	};
}

function harness() {
	const input = new FakeNode();
	const level = new FakeGain();
	const outputTrack = {
		stops: 0,
		stop() {
			this.stops++;
		},
	};
	const destination = Object.assign(new FakeNode(), {
		stream: { getTracks: () => [outputTrack] } as unknown as MediaStream,
	});
	const micTrack = {
		stops: 0,
		stop() {
			this.stops++;
		},
	};
	const microphone = { getTracks: () => [micTrack] } as unknown as MediaStream;
	const speakers = {};
	const sources: FakeBufferSource[] = [];
	const pcmBuffers: Float32Array[] = [];
	const buffer = { duration: 4 } as AudioBuffer;
	const speaking: (string | null)[] = [];
	const context = {
		state: "running",
		currentTime: 10,
		destination: speakers,
		decodeCalls: 0,
		resumeCalls: 0,
		decode: () => Promise.resolve(buffer),
		resumeResult: () => Promise.resolve(),
		startFailure: null as Error | null,
		createMediaStreamDestination: () => destination,
		createGain: () => level,
		createMediaStreamSource: (stream: MediaStream) => {
			expect(stream).toBe(microphone);
			return input;
		},
		decodeAudioData() {
			this.decodeCalls++;
			return this.decode();
		},
		resume() {
			this.resumeCalls++;
			return this.resumeResult();
		},
		createBuffer(_channels: number, length: number, sampleRate: number) {
			const samples = new Float32Array(length);
			pcmBuffers.push(samples);
			return {
				duration: length / sampleRate,
				getChannelData: () => samples,
			} as unknown as AudioBuffer;
		},
		createBufferSource() {
			const source = new FakeBufferSource();
			source.startFailure = this.startFailure;
			sources.push(source);
			return source;
		},
	};
	const output = createCallSpeechOutput(
		context as unknown as AudioContext,
		microphone,
		(botId) => speaking.push(botId),
	);
	const blob = { arrayBuffer: async () => new ArrayBuffer(1) } as Blob;
	return {
		output,
		context,
		input,
		level,
		destination,
		speakers,
		sources,
		pcmBuffers,
		outputTrack,
		micTrack,
		buffer,
		speaking,
		blob,
	};
}

/** The fake decoder and resume operation settle entirely through microtasks. */
async function settleAudio() {
	for (let index = 0; index < 40; index++) await Promise.resolve();
}

describe("createCallSpeechOutput", () => {
	test("sends the microphone only to peers and generated voice to peers and speakers", async () => {
		const h = harness();
		expect(h.output.stream).toBe(h.destination.stream);
		expect(h.input.connections).toEqual([h.destination]);
		const playing = h.output.play(h.blob, "bot-1");
		await settleAudio();
		const source = h.sources[0];
		expect(source?.buffer).toBe(h.buffer);
		expect(source?.connections).toEqual([h.level]);
		expect(h.level.connections).toEqual([h.destination, h.speakers]);
		expect(source?.starts).toBe(1);
		expect(h.speaking).toEqual(["bot-1"]);
		source?.onended?.();
		await playing;
		expect(h.speaking).toEqual(["bot-1", null]);
		expect(source?.disconnects).toBe(1);
		expect(source?.onended).toBeNull();
		h.output.dispose();
	});

	test("cancellation while decoding never starts stale speech", async () => {
		const h = harness();
		const decode = deferred<AudioBuffer>();
		h.context.decode = () => decode.promise;
		const playing = h.output.play(h.blob, "bot-1");
		await settleAudio();
		expect(h.context.decodeCalls).toBe(1);
		h.output.stop();
		decode.resolve(h.buffer);
		await playing;
		expect(h.sources).toHaveLength(0);
		expect(h.context.resumeCalls).toBe(0);
		expect(h.speaking).toEqual([]);
		h.output.dispose();
	});

	test("a replaced decode cannot play over the newer response", async () => {
		const h = harness();
		const staleDecode = deferred<AudioBuffer>();
		h.context.decode = () => staleDecode.promise;
		const stale = h.output.play(h.blob, "old-bot");
		await settleAudio();
		h.context.decode = () => Promise.resolve(h.buffer);
		const current = h.output.play(h.blob, "new-bot");
		await settleAudio();
		staleDecode.resolve(h.buffer);
		await stale;
		expect(h.sources).toHaveLength(1);
		expect(h.speaking).toEqual(["new-bot"]);
		expect(h.sources[0]?.stops).toBe(0);
		h.output.stop();
		await current;
		h.output.dispose();
	});

	test("cancellation while resuming cannot start speech", async () => {
		const h = harness();
		const resume = deferred<void>();
		h.context.resumeResult = () => resume.promise;
		const playing = h.output.play(h.blob, "bot-1");
		await settleAudio();
		expect(h.context.resumeCalls).toBe(1);
		h.output.stop();
		resume.resolve();
		await playing;
		expect(h.sources).toHaveLength(0);
		expect(h.speaking).toEqual([]);
		h.output.dispose();
	});

	test("stopping playback settles it and ignores a late ended event", async () => {
		const h = harness();
		const playing = h.output.play(h.blob, "bot-1");
		await settleAudio();
		const source = h.sources[0];
		if (!source) throw new Error("Speech did not start");
		const lateEnded = source.onended;
		h.output.stop();
		h.output.stop();
		lateEnded?.();
		await playing;
		expect(source.stops).toBe(1);
		expect(source.disconnects).toBe(1);
		expect(h.speaking).toEqual(["bot-1", null]);
		h.output.dispose();
	});

	test("disposing stops the mixed stream and leaves the microphone's lifetime to its owner", async () => {
		const h = harness();
		const playing = h.output.play(h.blob, "bot-1");
		await settleAudio();
		h.output.dispose();
		h.output.dispose();
		await playing;
		await h.output.play(h.blob, "bot-2");
		expect(h.input.disconnects).toBe(1);
		expect(h.destination.disconnects).toBe(1);
		expect(h.outputTrack.stops).toBe(1);
		expect(h.micTrack.stops).toBe(0);
		expect(h.context.decodeCalls).toBe(1);
		expect(h.sources[0]?.stops).toBe(1);
		expect(h.speaking).toEqual(["bot-1", null]);
	});

	test("disposing during decode prevents playback after resources are released", async () => {
		const h = harness();
		const decode = deferred<AudioBuffer>();
		h.context.decode = () => decode.promise;
		const playing = h.output.play(h.blob, "bot-1");
		await settleAudio();
		h.output.dispose();
		decode.resolve(h.buffer);
		await playing;
		expect(h.sources).toHaveLength(0);
		expect(h.outputTrack.stops).toBe(1);
	});

	test("playback start failure rejects and clears speaking state", async () => {
		const h = harness();
		h.context.startFailure = new Error("device unavailable");
		await expect(h.output.play(h.blob, "bot-1")).rejects.toThrow(
			"device unavailable",
		);
		expect(h.speaking).toEqual(["bot-1", null]);
		expect(h.sources[0]?.disconnects).toBe(1);
		expect(h.sources[0]?.onended).toBeNull();
		h.output.dispose();
	});

	test("decode and resume failures reject before announcing speech", async () => {
		for (const failure of ["decode", "resume", "suspended"]) {
			const h = harness();
			if (failure === "decode")
				h.context.decode = async () => {
					throw new Error("bad audio");
				};
			if (failure === "resume")
				h.context.resumeResult = async () => {
					throw new Error("device unavailable");
				};
			if (failure === "suspended") h.context.state = "suspended";
			await expect(h.output.play(h.blob, "bot-1")).rejects.toThrow();
			expect(h.sources).toHaveLength(0);
			expect(h.speaking).toEqual([]);
			h.output.dispose();
		}
	});

	test("reports playback progress from the audio clock", async () => {
		const h = harness();
		expect(h.output.progress()).toBeNull();
		const playing = h.output.play(h.blob, "bot-1");
		await settleAudio();
		h.context.currentTime = 11;
		expect(h.output.progress()).toEqual({ elapsedMs: 1000, durationMs: 4000 });
		h.context.currentTime = 30;
		expect(h.output.progress()?.elapsedMs).toBe(4000);
		h.output.stop();
		await playing;
		expect(h.output.progress()).toBeNull();
		h.output.dispose();
	});

	test("ducking lowers and restores the shared output level", () => {
		const h = harness();
		h.output.duck(true);
		h.output.duck(false);
		expect(h.level.targets.map((target) => target.value)).toEqual([0.2, 1]);
		h.output.dispose();
		h.output.duck(true);
		expect(h.level.targets).toHaveLength(2);
		expect(h.level.disconnects).toBe(1);
	});

	test("pausing holds the position and resuming restarts from it", async () => {
		const h = harness();
		expect(h.output.pause()).toBe(false);
		const playing = h.output.play(h.blob, "bot-1");
		await settleAudio();
		h.context.currentTime = 11;
		expect(h.output.pause()).toBe(true);
		expect(h.output.pause()).toBe(false);
		expect(h.sources[0]?.stops).toBe(1);
		expect(h.speaking).toEqual(["bot-1"]);
		h.context.currentTime = 20;
		expect(h.output.progress()).toEqual({ elapsedMs: 1000, durationMs: 4000 });
		h.output.resume();
		expect(h.sources).toHaveLength(2);
		expect(h.sources[1]?.startArgs[0]).toEqual([20, 1]);
		h.context.currentTime = 22;
		expect(h.output.progress()).toEqual({ elapsedMs: 3000, durationMs: 4000 });
		h.sources[1]?.onended?.();
		await playing;
		expect(h.speaking).toEqual(["bot-1", null]);
		h.output.dispose();
	});

	test("stopping while paused ends the speech", async () => {
		const h = harness();
		const playing = h.output.play(h.blob, "bot-1");
		await settleAudio();
		h.output.pause();
		h.output.stop();
		await playing;
		h.output.resume();
		expect(h.sources).toHaveLength(1);
		expect(h.speaking).toEqual(["bot-1", null]);
		h.output.dispose();
	});

	function bytes(...values: number[]) {
		return new Uint8Array(values);
	}

	test("streamed PCM keeps an odd trailing byte for the next chunk and plays gaplessly", async () => {
		const h = harness();
		const playing = h.output.playStream(
			{
				sampleRate: 1000,
				chunks: (async function* () {
					yield bytes(0x00, 0x40, 0x00);
					yield bytes(0xc0);
					yield bytes();
					yield bytes(0x00, 0x20);
				})(),
			},
			"bot-1",
		);
		await settleAudio();
		expect(h.pcmBuffers.map((samples) => [...samples])).toEqual([
			[0.5],
			[-0.5],
			[0.25],
		]);
		expect(h.sources).toHaveLength(3);
		expect(h.speaking).toEqual(["bot-1"]);
		expect(h.sources.map((source) => source.startArgs[0])).toEqual([
			[10, 0],
			[10.001, 0],
			[10.002, 0],
		]);
		for (const source of h.sources) source.onended?.();
		await playing;
		expect(h.speaking).toEqual(["bot-1", null]);
		h.output.dispose();
	});

	test("streamed speech starts once about 120 ms is buffered and follows late chunks", async () => {
		const h = harness();
		const gate = deferred<void>();
		const playing = h.output.playStream(
			{
				sampleRate: 1000,
				chunks: (async function* () {
					yield new Uint8Array(100);
					yield new Uint8Array(100);
					await gate.promise;
					yield new Uint8Array(40);
				})(),
			},
			"bot-1",
		);
		await settleAudio();
		expect(h.pcmBuffers).toHaveLength(2);
		expect(h.sources).toHaveLength(0);
		expect(h.speaking).toEqual([]);
		expect(h.output.progress()).toBeNull();
		gate.resolve();
		await settleAudio();
		expect(h.sources).toHaveLength(3);
		expect(h.speaking).toEqual(["bot-1"]);
		h.output.stop();
		await playing;
		h.output.dispose();
	});

	test("a stream that reaches 120 ms starts before it ends, and a late chunk is rebased after an underrun", async () => {
		const h = harness();
		const gate = deferred<void>();
		const playing = h.output.playStream(
			{
				sampleRate: 1000,
				chunks: (async function* () {
					yield new Uint8Array(260);
					await gate.promise;
					yield new Uint8Array(20);
				})(),
			},
			"bot-1",
		);
		await settleAudio();
		expect(h.sources).toHaveLength(1);
		expect(h.speaking).toEqual(["bot-1"]);
		expect(h.output.progress()).toEqual({ elapsedMs: 0, durationMs: 130 });
		h.context.currentTime = 11;
		gate.resolve();
		await settleAudio();
		expect(h.sources[1]?.startArgs[0]).toEqual([11, 0]);
		for (const source of [...h.sources]) source.onended?.();
		await playing;
		expect(h.speaking).toEqual(["bot-1", null]);
		h.output.dispose();
	});

	test("pausing a stream keeps reading and resume schedules everything after the position", async () => {
		const h = harness();
		const gate = deferred<void>();
		const playing = h.output.playStream(
			{
				sampleRate: 1000,
				chunks: (async function* () {
					yield new Uint8Array(400);
					await gate.promise;
					yield new Uint8Array(200);
				})(),
			},
			"bot-1",
		);
		await settleAudio();
		h.context.currentTime = 10.1;
		expect(h.output.pause()).toBe(true);
		gate.resolve();
		await settleAudio();
		expect(h.sources).toHaveLength(1);
		expect(h.output.progress()?.durationMs).toBeCloseTo(300);
		h.context.currentTime = 15;
		h.output.resume();
		expect(h.sources).toHaveLength(3);
		expect(h.sources[1]?.startArgs[0]?.[0]).toBe(15);
		expect(h.sources[1]?.startArgs[0]?.[1]).toBeCloseTo(0.1);
		expect(h.sources[2]?.startArgs[0]?.[0]).toBeCloseTo(15.1);
		for (const source of h.sources.slice(1)) source.onended?.();
		await playing;
		h.output.dispose();
	});

	test("stopping a stream ends it and releases the reader", async () => {
		const h = harness();
		let returned = 0;
		const playing = h.output.playStream(
			{
				sampleRate: 1000,
				chunks: {
					[Symbol.asyncIterator]: () => ({
						next: () => new Promise<IteratorResult<Uint8Array>>(() => {}),
						return: async () => {
							returned++;
							return { done: true, value: undefined };
						},
					}),
				},
			},
			"bot-1",
		);
		await settleAudio();
		h.output.stop();
		await playing;
		expect(returned).toBe(1);
		expect(h.speaking).toEqual([]);
		h.output.dispose();
	});

	test("a failing stream rejects and clears the speaking state", async () => {
		const h = harness();
		const playing = h.output.playStream(
			{
				sampleRate: 1000,
				chunks: (async function* () {
					yield new Uint8Array(400);
					throw new Error("connection lost");
				})(),
			},
			"bot-1",
		);
		await expect(playing).rejects.toThrow("connection lost");
		expect(h.speaking).toEqual(["bot-1", null]);
		h.output.dispose();
	});

	test("the output exposes its cues for the local speakers", () => {
		const h = harness();
		expect(typeof h.output.cue.thinking.start).toBe("function");
		h.output.dispose();
	});
});
