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

afterEach(() => vi.unstubAllGlobals());

describe("call microphone capture", () => {
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
