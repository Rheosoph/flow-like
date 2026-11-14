import { afterEach, describe, expect, test, vi } from "vitest";
import { VAD_WAIT_MS, startCallListener } from "./call-listener";

function context() {
	const source = { connect: vi.fn(), disconnect: vi.fn() };
	const sink = {
		gain: { value: 1 },
		connect: vi.fn(),
		disconnect: vi.fn(),
	};
	const processor = {
		connect: vi.fn(),
		disconnect: vi.fn(),
		onaudioprocess: null as ((event: AudioProcessingEvent) => void) | null,
	};
	const api = {
		state: "running",
		sampleRate: 16_000,
		destination: {},
		resume: vi.fn(async () => {}),
		close: vi.fn(async () => {}),
		createMediaStreamSource: vi.fn(() => source),
		createGain: vi.fn(() => sink),
		createScriptProcessor: vi.fn(() => processor),
	};
	return {
		api,
		source,
		sink,
		processor,
		context: api as unknown as AudioContext,
		block(level: number) {
			processor.onaudioprocess?.({
				inputBuffer: {
					getChannelData: () => new Float32Array(2048).fill(level),
				},
			} as unknown as AudioProcessingEvent);
		},
	};
}

afterEach(() => {
	vi.useRealTimers();
	vi.unstubAllGlobals();
	vi.restoreAllMocks();
});

describe("call microphone capture", () => {
	test("previews speech before silence and preserves its capture time on final delivery", async () => {
		const audio = context();
		let now = 10_000;
		vi.spyOn(Date, "now").mockImplementation(() => now);
		const onUtterance = vi.fn();
		const onPartial = vi.fn();
		const onReset = vi.fn();
		const listener = await startCallListener({} as MediaStream, onUtterance, {
			context: audio.context,
			onPartial,
			onReset,
		});
		for (let index = 0; index < 9; index++) {
			now += 128;
			audio.block(0.1);
		}
		expect(onPartial).toHaveBeenCalledOnce();
		expect(onPartial.mock.calls[0][0]).toMatchObject({
			offsetMs: 0,
			durationMs: 1024,
			capturedAt: 10_000,
		});
		expect(onUtterance).not.toHaveBeenCalled();
		for (let index = 0; index < 6; index++) {
			now += 128;
			audio.block(0);
		}
		expect(onPartial).toHaveBeenCalledTimes(2);
		expect(onPartial.mock.calls[1][0]).toMatchObject({
			settled: true,
			offsetMs: 0,
			durationMs: onUtterance.mock.calls[0][0].durationMs,
		});
		expect(onUtterance).toHaveBeenCalledOnce();
		expect(onUtterance.mock.calls[0][0]).toMatchObject({
			durationMs: 1302,
			capturedAt: 10_000,
		});
		listener.stop();
		listener.stop();
		expect(onReset).toHaveBeenCalledOnce();
	});

	test("pausing invalidates previews and anchors resumed speech after the pause", async () => {
		const audio = context();
		let now = 10_000;
		vi.spyOn(Date, "now").mockImplementation(() => now);
		const onUtterance = vi.fn();
		const onPartial = vi.fn();
		const onReset = vi.fn();
		const listener = await startCallListener({} as MediaStream, onUtterance, {
			context: audio.context,
			onPartial,
			onReset,
		});
		for (let index = 0; index < 8; index++) {
			now += 128;
			audio.block(0.1);
		}
		listener.setPaused(true);
		listener.setPaused(true);
		expect(onReset).toHaveBeenCalledOnce();
		for (let index = 0; index < 20; index++) audio.block(0.1);
		expect(onPartial).toHaveBeenCalledOnce();
		listener.setPaused(false);
		now = 30_000;
		for (let index = 0; index < 8; index++) {
			now += 128;
			audio.block(0.2);
		}
		expect(onPartial).toHaveBeenCalledTimes(2);
		expect(onPartial.mock.calls[1][0]).toMatchObject({
			offsetMs: 1024,
			durationMs: 1024,
			capturedAt: 30_000,
		});
		listener.stop();
		expect(onUtterance).toHaveBeenCalledOnce();
		expect(onUtterance.mock.calls[0][0]).toMatchObject({
			capturedAt: 30_000,
		});
		expect(onReset).toHaveBeenCalledTimes(2);
	});

	test("re-enabling listening registers its worklet once per call context", async () => {
		const audio = context();
		const addModule = vi.fn(async () => {});
		Object.assign(audio.api, { audioWorklet: { addModule } });
		vi.stubGlobal(
			"AudioWorkletNode",
			class {
				port = { onmessage: null };
				connect = vi.fn();
				disconnect = vi.fn();
			},
		);
		for (let index = 0; index < 2; index++) {
			const listener = await startCallListener({} as MediaStream, vi.fn(), {
				context: audio.context,
			});
			listener.stop();
		}
		expect(addModule).toHaveBeenCalledOnce();
		expect(audio.api.createScriptProcessor).not.toHaveBeenCalled();
		expect(audio.api.close).not.toHaveBeenCalled();
	});

	test("a failed worklet registration can be retried on the same context", async () => {
		const audio = context();
		const addModule = vi
			.fn(async () => {})
			.mockRejectedValueOnce(new Error("module failed"));
		Object.assign(audio.api, { audioWorklet: { addModule } });
		vi.stubGlobal(
			"AudioWorkletNode",
			class {
				port = { onmessage: null };
				connect = vi.fn();
				disconnect = vi.fn();
			},
		);
		for (let index = 0; index < 2; index++) {
			const listener = await startCallListener({} as MediaStream, vi.fn(), {
				context: audio.context,
			});
			listener.stop();
		}
		expect(addModule).toHaveBeenCalledTimes(2);
		expect(audio.api.createScriptProcessor).toHaveBeenCalledOnce();
	});

	test("captures speech on the call context and leaves it running when listening stops", async () => {
		const audio = context();
		const onUtterance = vi.fn();
		const listener = await startCallListener({} as MediaStream, onUtterance, {
			context: audio.context,
			paused: true,
		});
		for (let index = 0; index < 6; index++) audio.block(0.1);
		for (let index = 0; index < 6; index++) audio.block(0);
		expect(onUtterance).not.toHaveBeenCalled();
		listener.setPaused(false);
		for (let index = 0; index < 6; index++) audio.block(0.1);
		for (let index = 0; index < 6; index++) audio.block(0);
		expect(onUtterance).toHaveBeenCalledOnce();
		listener.stop();
		listener.stop();
		expect(audio.source.disconnect).toHaveBeenCalledOnce();
		expect(audio.processor.disconnect).toHaveBeenCalledOnce();
		expect(audio.sink.disconnect).toHaveBeenCalledOnce();
		expect(audio.api.close).not.toHaveBeenCalled();
	});

	test("a failed standalone context resume rejects capture and releases its context", async () => {
		const audio = context();
		audio.api.state = "suspended";
		audio.api.resume.mockRejectedValue(new Error("audio denied"));
		vi.stubGlobal(
			"AudioContext",
			class {
				constructor() {
					Object.assign(this, audio.api);
				}
			},
		);
		await expect(startCallListener({} as MediaStream, vi.fn())).rejects.toThrow(
			"audio denied",
		);
		expect(audio.api.close).toHaveBeenCalledOnce();
		expect(audio.api.createMediaStreamSource).not.toHaveBeenCalled();
	});

	test("failed capture setup disconnects its source and sink without closing call audio", async () => {
		const audio = context();
		audio.api.createScriptProcessor.mockImplementation(() => {
			throw new Error("capture unavailable");
		});
		await expect(
			startCallListener({} as MediaStream, vi.fn(), { context: audio.context }),
		).rejects.toThrow("capture unavailable");
		expect(audio.source.disconnect).toHaveBeenCalledOnce();
		expect(audio.sink.disconnect).toHaveBeenCalledOnce();
		expect(audio.api.close).not.toHaveBeenCalled();
	});

	describe("barge-in", () => {
		async function guardedListener() {
			const audio = context();
			let now = 10_000;
			vi.spyOn(Date, "now").mockImplementation(() => now);
			const onUtterance = vi.fn();
			const onPartial = vi.fn();
			const onBargeInStart = vi.fn();
			const onBargeInProbe = vi.fn();
			const listener = await startCallListener({} as MediaStream, onUtterance, {
				context: audio.context,
				guarded: true,
				onPartial,
				onBargeInStart,
				onBargeInProbe,
			});
			const blocks = (count: number, level: number) => {
				for (let index = 0; index < count; index++) {
					now += 128;
					audio.block(level);
				}
			};
			return {
				listener,
				blocks,
				onUtterance,
				onPartial,
				onBargeInStart,
				onBargeInProbe,
				advance: (ms: number) => {
					now += ms;
				},
			};
		}

		test("ordinary speech and bot echo below the guard threshold are ignored", async () => {
			const h = await guardedListener();
			h.blocks(10, 0.03);
			h.blocks(6, 0);
			expect(h.onBargeInStart).not.toHaveBeenCalled();
			expect(h.onUtterance).not.toHaveBeenCalled();
			expect(h.onPartial).not.toHaveBeenCalled();
		});

		test("sustained loud audio starts a barge-in, probes once enough is captured and keeps the utterance when confirmed", async () => {
			const h = await guardedListener();
			h.blocks(2, 0.1);
			h.blocks(1, 0);
			expect(h.onBargeInStart).not.toHaveBeenCalled();
			h.blocks(1, 0.1);
			expect(h.onBargeInStart).toHaveBeenCalledOnce();
			expect(h.onBargeInProbe).toHaveBeenCalledOnce();
			const probe = h.onBargeInProbe.mock.calls[0][0];
			expect(probe.durationMs).toBeGreaterThanOrEqual(500);
			expect(probe.samples.length).toBe((probe.durationMs * 16_000) / 1000);
			h.blocks(2, 0.1);
			h.blocks(8, 0);
			expect(h.onUtterance).not.toHaveBeenCalled();
			h.listener.endBargeIn(true);
			expect(h.onUtterance).toHaveBeenCalledOnce();
			h.blocks(4, 0.03);
			h.blocks(6, 0);
			expect(h.onUtterance).toHaveBeenCalledTimes(2);
			expect(h.onBargeInStart).toHaveBeenCalledOnce();
		});

		test("a rejected barge-in drops the audio and is not retried during the cooldown", async () => {
			const h = await guardedListener();
			h.blocks(8, 0.1);
			expect(h.onBargeInStart).toHaveBeenCalledOnce();
			h.listener.endBargeIn(false);
			h.blocks(8, 0.1);
			h.blocks(8, 0);
			expect(h.onBargeInStart).toHaveBeenCalledOnce();
			expect(h.onUtterance).not.toHaveBeenCalled();
			h.advance(3000);
			h.blocks(4, 0.1);
			expect(h.onBargeInStart).toHaveBeenCalledTimes(2);
		});

		test("one empty transcript can request a longer probe without discarding the utterance", async () => {
			const h = await guardedListener();
			expect(h.listener.retryBargeIn()).toBe(false);
			h.blocks(4, 0.1);
			expect(h.onBargeInProbe).toHaveBeenCalledOnce();
			const first = h.onBargeInProbe.mock.calls[0][0];
			expect(h.listener.retryBargeIn()).toBe(true);
			h.blocks(3, 0.1);
			expect(h.onBargeInProbe).toHaveBeenCalledOnce();
			h.blocks(1, 0.1);
			expect(h.onBargeInProbe).toHaveBeenCalledTimes(2);
			const second = h.onBargeInProbe.mock.calls[1][0];
			expect(second.durationMs).toBeGreaterThanOrEqual(1000);
			expect(second.capturedAt).toBe(first.capturedAt);
			expect(second.samples.slice(0, first.samples.length)).toEqual(
				first.samples,
			);
			expect(h.listener.retryBargeIn()).toBe(false);
			h.blocks(4, 0);
			h.listener.endBargeIn(true);
			expect(h.onUtterance).toHaveBeenCalledOnce();
			expect(h.onBargeInStart).toHaveBeenCalledOnce();
			h.listener.stop();
		});

		test("a slow first transcript retries immediately with the audio captured while waiting", async () => {
			const h = await guardedListener();
			h.blocks(10, 0.1);
			expect(h.onBargeInProbe).toHaveBeenCalledOnce();
			expect(h.listener.retryBargeIn()).toBe(true);
			expect(h.onBargeInProbe).toHaveBeenCalledTimes(2);
			expect(h.onBargeInProbe.mock.calls[1][0].durationMs).toBe(1280);
			h.listener.endBargeIn(false);
			expect(h.listener.retryBargeIn()).toBe(false);
			h.listener.stop();
		});

		test("a retry still completes if microphone frames stop arriving", async () => {
			vi.useFakeTimers();
			const h = await guardedListener();
			h.blocks(4, 0.1);
			expect(h.listener.retryBargeIn()).toBe(true);
			vi.advanceTimersByTime(1000);
			expect(h.onBargeInProbe).toHaveBeenCalledTimes(2);
			expect(h.listener.retryBargeIn()).toBe(false);
			h.listener.stop();
		});

		test("capture stalling before the first probe cannot leave confirmation waiting forever", async () => {
			vi.useFakeTimers();
			const h = await guardedListener();
			h.blocks(3, 0.1);
			expect(h.onBargeInStart).toHaveBeenCalledOnce();
			expect(h.onBargeInProbe).not.toHaveBeenCalled();
			vi.advanceTimersByTime(500);
			expect(h.onBargeInProbe).toHaveBeenCalledOnce();
			expect(h.onBargeInProbe.mock.calls[0][0].durationMs).toBe(384);
			h.listener.endBargeIn(false);
			h.listener.stop();
		});

		test.each(["mute", "reject", "stop"])(
			"%s discards a pending probe retry",
			async (action) => {
				vi.useFakeTimers();
				const h = await guardedListener();
				h.blocks(4, 0.1);
				expect(h.listener.retryBargeIn()).toBe(true);
				if (action === "mute") h.listener.setPaused(true);
				else if (action === "reject") h.listener.endBargeIn(false);
				else h.listener.stop();
				vi.advanceTimersByTime(2000);
				expect(h.onBargeInProbe).toHaveBeenCalledOnce();
				expect(h.listener.retryBargeIn()).toBe(false);
				h.listener.stop();
			},
		);

		test("pausing ends a barge-in, so a muted person never interrupts", async () => {
			const h = await guardedListener();
			h.listener.setPaused(true);
			h.blocks(10, 0.1);
			expect(h.onBargeInStart).not.toHaveBeenCalled();
			h.listener.setPaused(false);
			h.blocks(4, 0.1);
			expect(h.onBargeInStart).toHaveBeenCalledOnce();
			h.listener.setPaused(true);
			h.listener.endBargeIn(true);
			h.blocks(8, 0);
			expect(h.onUtterance).not.toHaveBeenCalled();
		});

		test("lifting the guard resumes normal capture", async () => {
			const h = await guardedListener();
			h.blocks(2, 0.03);
			h.listener.setGuarded(false);
			h.blocks(8, 0.03);
			h.blocks(6, 0);
			expect(h.onUtterance).toHaveBeenCalledOnce();
		});
	});
});

describe("call microphone capture with voice-activity and turn models", () => {
	const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

	function models(
		options: {
			speech?: (level: number, index: number) => Promise<number | null>;
			turn?: () => Promise<number | null>;
			ready?: boolean;
		} = {},
	) {
		let index = 0;
		let level = 0;
		const detector = {
			ready: options.ready ?? true,
			turnReady: true,
			speech: vi.fn(() => {
				const current = index++;
				const heard = level;
				return (
					options.speech?.(heard, current) ??
					Promise.resolve(heard > 0 ? 0.9 : 0.02)
				);
			}),
			resetSpeech: vi.fn(),
			turnComplete: vi.fn(options.turn ?? (() => Promise.resolve(0.9))),
		};
		return {
			detector,
			setLevel: (next: number) => {
				level = next;
			},
		};
	}

	async function listen(
		detector: ReturnType<typeof models>["detector"] | null,
		extra: { guarded?: boolean } = {},
	) {
		const audio = context();
		let now = 10_000;
		vi.spyOn(Date, "now").mockImplementation(() => now);
		const onUtterance = vi.fn();
		const onBargeInStart = vi.fn();
		const onBargeInProbe = vi.fn();
		const onPartial = vi.fn();
		const listener = await startCallListener({} as MediaStream, onUtterance, {
			context: audio.context,
			detector,
			onPartial,
			onBargeInStart,
			onBargeInProbe,
			guarded: extra.guarded,
		});
		return {
			listener,
			onUtterance,
			onBargeInStart,
			onBargeInProbe,
			onPartial,
			push(level: number) {
				now += 128;
				audio.block(level);
			},
			async blocks(count: number, level: number, hear?: (n: number) => void) {
				for (let i = 0; i < count; i++) {
					now += 128;
					hear?.(level);
					audio.block(level);
					await settle();
				}
			},
		};
	}

	test("a short 'Ja' is kept when the models hear it, and ends as soon as the turn check says so", async () => {
		const m = models();
		const h = await listen(m.detector);
		await h.blocks(2, 0.3, m.setLevel);
		await h.blocks(2, 0, m.setLevel);
		await settle();
		expect(m.detector.turnComplete).toHaveBeenCalledOnce();
		expect(h.onUtterance).toHaveBeenCalledOnce();
		const [utterance] = h.onUtterance.mock.calls[0];
		expect(utterance.turn).toMatchObject({ source: "model", probability: 0.9 });
		expect(utterance.capturedAt).toBe(10_000);
		expect(utterance.endOfSpeechAt).toBe(10_000 + 256);
		const settled = h.onPartial.mock.calls
			.map(([u]) => u)
			.find((u) => u.settled);
		expect(settled.durationMs).toBe(utterance.durationMs);
		expect(settled.samples).toEqual(utterance.samples);
	});

	test("the same 'Ja' is dropped on the energy rule", async () => {
		const h = await listen(null);
		await h.blocks(2, 0.3);
		await h.blocks(6, 0);
		expect(h.onUtterance).not.toHaveBeenCalled();
	});

	test("an incomplete turn waits for the long silence", async () => {
		const m = models({ turn: () => Promise.resolve(0.1) });
		const h = await listen(m.detector);
		await h.blocks(6, 0.3, m.setLevel);
		await h.blocks(4, 0, m.setLevel);
		expect(h.onUtterance).not.toHaveBeenCalled();
		await h.blocks(7, 0, m.setLevel);
		expect(h.onUtterance).not.toHaveBeenCalled();
		await h.blocks(1, 0, m.setLevel);
		expect(h.onUtterance).toHaveBeenCalledOnce();
		expect(h.onUtterance.mock.calls[0][0].turn).toMatchObject({
			source: "silence",
			probability: 0.1,
		});
	});

	test("blocks are handled in order while inference is delayed", async () => {
		const resolvers: ((p: number) => void)[] = [];
		const m = models({
			speech: () => new Promise<number>((resolve) => resolvers.push(resolve)),
		});
		const h = await listen(m.detector);
		await h.blocks(3, 0.3);
		await h.blocks(3, 0);
		expect(resolvers).toHaveLength(6);
		expect(h.onPartial).not.toHaveBeenCalled();
		expect(m.detector.turnComplete).not.toHaveBeenCalled();
		[0.9, 0.9, 0.9, 0.02, 0.02, 0.02].forEach((p, i) => resolvers[i](p));
		await settle();
		expect(m.detector.turnComplete).toHaveBeenCalledOnce();
		await settle();
		expect(h.onUtterance).toHaveBeenCalledOnce();
		const [utterance] = h.onUtterance.mock.calls[0];
		expect(utterance.offsetMs).toBe(0);
		expect(utterance.durationMs).toBe(384 + 150);
		expect(utterance.capturedAt).toBe(10_000);
	});

	test("a dead worker never stalls capture, the energy rule takes over", async () => {
		const m = models({ speech: () => new Promise<number | null>(() => {}) });
		vi.useFakeTimers();
		try {
			const h = await listen(m.detector);
			for (const level of [...Array(6).fill(0.1), ...Array(6).fill(0)]) {
				h.push(level);
				await vi.advanceTimersByTimeAsync(VAD_WAIT_MS);
			}
			expect(m.detector.speech).toHaveBeenCalledTimes(12);
			expect(h.onUtterance).toHaveBeenCalledOnce();
			expect(h.onUtterance.mock.calls[0][0].turn?.source).toBe("silence");
		} finally {
			vi.useRealTimers();
		}
	});

	test("a model that is not ready is not asked, and capture stays on energy", async () => {
		const m = models({ ready: false });
		const h = await listen(m.detector);
		await h.blocks(6, 0.1);
		await h.blocks(6, 0);
		expect(m.detector.speech).not.toHaveBeenCalled();
		expect(h.onUtterance).toHaveBeenCalledOnce();
		expect(h.onUtterance.mock.calls[0][0].turn?.source).toBe("silence");
	});

	test("a model that says null for a block leaves that block to the energy rule", async () => {
		const m = models({ speech: () => Promise.resolve(null) });
		const h = await listen(m.detector);
		await h.blocks(6, 0.1);
		await h.blocks(6, 0);
		expect(h.onUtterance).toHaveBeenCalledOnce();
		expect(h.onUtterance.mock.calls[0][0].turn?.source).toBe("silence");
	});

	test("stopping resets the model's stream for this listener", async () => {
		const m = models();
		const h = await listen(m.detector);
		h.listener.stop();
		expect(m.detector.resetSpeech).toHaveBeenCalled();
	});

	describe("barge-in", () => {
		test("loud audio the model does not call speech never interrupts", async () => {
			const m = models({ speech: () => Promise.resolve(0.1) });
			const h = await listen(m.detector, { guarded: true });
			await h.blocks(12, 0.3);
			expect(h.onBargeInStart).not.toHaveBeenCalled();
		});

		test("about 250 ms of loud speech interrupts at once", async () => {
			const m = models({ speech: () => Promise.resolve(0.9) });
			const h = await listen(m.detector, { guarded: true });
			await h.blocks(1, 0.3);
			expect(h.onBargeInStart).not.toHaveBeenCalled();
			await h.blocks(1, 0.3);
			expect(h.onBargeInStart).toHaveBeenCalledOnce();
		});

		test("attenuated speech reaches confirmation and preserves the entire interruption", async () => {
			const m = models();
			const h = await listen(m.detector, { guarded: true });
			await h.blocks(1, 0.01, m.setLevel);
			expect(h.onBargeInStart).not.toHaveBeenCalled();
			await h.blocks(1, 0.01, m.setLevel);
			expect(h.onBargeInStart).toHaveBeenCalledOnce();
			expect(h.onBargeInProbe).not.toHaveBeenCalled();
			await h.blocks(2, 0.01, m.setLevel);
			expect(h.onBargeInProbe).toHaveBeenCalledOnce();
			const probe = h.onBargeInProbe.mock.calls[0][0];
			expect(probe.capturedAt).toBe(10_000);
			expect(probe.samples.length).toBe(4 * 2048);
			await h.blocks(2, 0.01, m.setLevel);
			await h.blocks(4, 0, m.setLevel);
			expect(h.onUtterance).not.toHaveBeenCalled();
			h.listener.endBargeIn(true);
			expect(h.onUtterance).toHaveBeenCalledOnce();
			const utterance = h.onUtterance.mock.calls[0][0];
			expect(utterance.capturedAt).toBe(probe.capturedAt);
			expect(utterance.samples.slice(0, 6 * 2048)).toEqual(
				new Float32Array(6 * 2048).fill(0.01),
			);
			h.listener.stop();
		});

		test("a short quiet speech burst cannot pause the bot", async () => {
			const m = models();
			const h = await listen(m.detector, { guarded: true });
			await h.blocks(1, 0.01, m.setLevel);
			await h.blocks(6, 0, m.setLevel);
			expect(h.onBargeInStart).not.toHaveBeenCalled();
			expect(h.onBargeInProbe).not.toHaveBeenCalled();
			h.listener.stop();
		});

		test("without models the loud-only rule still needs 300 ms", async () => {
			const h = await listen(null, { guarded: true });
			await h.blocks(2, 0.3);
			expect(h.onBargeInStart).not.toHaveBeenCalled();
			await h.blocks(1, 0.3);
			expect(h.onBargeInStart).toHaveBeenCalledOnce();
		});
	});
});
