import { afterEach, describe, expect, test, vi } from "vitest";
import { startCallListener } from "./call-listener";

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
		expect(onPartial).toHaveBeenCalledOnce();
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
});
