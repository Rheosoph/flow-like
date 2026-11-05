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
	start() {
		this.starts++;
		if (this.startFailure) throw this.startFailure;
	}
	stop() {
		this.stops++;
	}
}

function harness() {
	const input = new FakeNode();
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
	const buffer = {} as AudioBuffer;
	const speaking: (string | null)[] = [];
	const context = {
		state: "running",
		destination: speakers,
		decodeCalls: 0,
		resumeCalls: 0,
		decode: () => Promise.resolve(buffer),
		resumeResult: () => Promise.resolve(),
		startFailure: null as Error | null,
		createMediaStreamDestination: () => destination,
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
		destination,
		speakers,
		sources,
		outputTrack,
		micTrack,
		buffer,
		speaking,
		blob,
	};
}

/** The fake decoder and resume operation settle entirely through microtasks. */
async function settleAudio() {
	for (let index = 0; index < 8; index++) await Promise.resolve();
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
		expect(source?.connections).toEqual([h.destination, h.speakers]);
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
});
