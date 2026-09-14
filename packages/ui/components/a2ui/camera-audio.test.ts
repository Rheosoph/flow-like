import { describe, expect, test } from "bun:test";
import {
	cameraAudioWorkletSource,
	createCameraPcmRing,
	encodeCameraWav,
	validateAudioBufferSeconds,
	validateAudioDuration,
} from "./camera-audio";

describe("rolling camera PCM", () => {
	test("a ten-second request after five seconds returns five seconds of mono 16 kHz PCM", () => {
		const ring = createCameraPcmRing(48_000, 60);
		const left = new Float32Array(48_000 * 5).fill(0.75);
		const right = new Float32Array(left.length).fill(0.25);
		for (let i = 0; i < left.length; i += 128)
			ring.append(
				[left.subarray(i, i + 128), right.subarray(i, i + 128)],
				i / 48_000,
			);
		const snapshot = ring.snapshot(10_000, 5);
		expect(snapshot.pcm.length).toBe(80_000);
		expect(snapshot.endTime).toBe(5);
		expect(snapshot.pcm.every((sample) => sample === 0.5)).toBe(true);
		expect(ring.availableMs()).toBe(5000);
	});

	test("a wrapped ring evicts old samples and slices the exact requested tail", () => {
		const ring = createCameraPcmRing(16_000, 2);
		for (let second = 0; second < 6; second++)
			ring.append([new Float32Array(16_000).fill((second + 1) / 10)], second);
		const full = ring.snapshot(10_000, 6);
		expect(full.pcm.length).toBe(32_000);
		expect(full.pcm[0]).toBeCloseTo(0.5);
		expect(full.pcm[16_000]).toBeCloseTo(0.6);
		const tail = ring.snapshot(100.0625, 6);
		expect(tail.pcm.length).toBe(1601);
		expect(tail.endTime).toBe(6);
		expect(tail.pcm[0]).toBeCloseTo(0.6);
		expect(ring.availableMs()).toBe(2000);
	});

	test("the command's cutoff excludes audio recorded after its camera frame", () => {
		const ring = createCameraPcmRing(16_000, 5);
		for (let second = 0; second < 4; second++)
			ring.append([new Float32Array(16_000).fill((second + 1) / 10)], second);
		const snapshot = ring.snapshot(1000, 3);
		expect(snapshot.endTime).toBe(3);
		expect(snapshot.pcm.length).toBe(16_000);
		expect(snapshot.pcm[0]).toBeCloseTo(0.3);
		expect(snapshot.pcm.at(-1)).toBeCloseTo(0.3);
	});

	test("44.1 kHz resampling keeps fractional phase across render quanta", () => {
		const ring = createCameraPcmRing(44_100, 1);
		const input = new Float32Array(44_100).fill(0.25);
		for (let i = 0; i < input.length; i += 128)
			ring.append([input.subarray(i, i + 128)], i / 44_100);
		expect(ring.snapshot(1000, 1).pcm.length).toBe(16_000);
		ring.clear();
		expect(ring.snapshot(1000, 1).pcm.length).toBe(0);
		ring.append([new Float32Array(4410).fill(-0.5)], 20);
		expect(ring.snapshot(1000, 20.1).pcm.length).toBe(1600);
		expect(ring.snapshot(1000, 20.1).endTime).toBe(20.1);
	});

	test("exports a complete little-endian PCM16 WAV with truthful sizes and clipping", () => {
		const bytes = encodeCameraWav(
			new Float32Array([-2, -0.5, 0, 0.5, 2, Number.NaN]),
		);
		const view = new DataView(bytes);
		const text = (start: number, length: number) =>
			new TextDecoder().decode(bytes.slice(start, start + length));
		expect(text(0, 4)).toBe("RIFF");
		expect(text(8, 8)).toBe("WAVEfmt ");
		expect(text(36, 4)).toBe("data");
		expect(bytes.byteLength).toBe(56);
		expect(view.getUint32(4, true)).toBe(48);
		expect(view.getUint16(20, true)).toBe(1);
		expect(view.getUint16(22, true)).toBe(1);
		expect(view.getUint32(24, true)).toBe(16000);
		expect(view.getUint32(28, true)).toBe(32000);
		expect(view.getUint16(32, true)).toBe(2);
		expect(view.getUint16(34, true)).toBe(16);
		expect(view.getUint32(40, true)).toBe(12);
		expect(
			Array.from({ length: 6 }, (_, i) => view.getInt16(44 + i * 2, true)),
		).toEqual([-32768, -16384, 0, 16384, 32767, 0]);
	});

	test("rejects unbounded or fractional history and invalid requested durations", () => {
		for (const value of [0, 301, 1.5, Number.NaN])
			expect(() => validateAudioBufferSeconds(value)).toThrow();
		for (const value of [99, 300_001, Number.NaN, Number.POSITIVE_INFINITY])
			expect(() => validateAudioDuration(value)).toThrow();
		expect(validateAudioBufferSeconds(300)).toBe(300);
		expect(validateAudioDuration(300_000)).toBe(300_000);
	});

	test("the shipped worklet records silently, clears on pause, and resumes with empty history", () => {
		const messages: { pcm: Float32Array; epoch: number }[] = [];
		let Processor!: new (options: {
			processorOptions: { bufferSeconds: number };
		}) => BaseProcessor & {
			process(inputs: Float32Array[][], outputs: Float32Array[][]): boolean;
		};
		class BaseProcessor {
			port = {
				onmessage: (_event: { data: Record<string, unknown> }) => {},
				postMessage: (message: { pcm: Float32Array; epoch: number }) =>
					messages.push(message),
			};
		}
		const clock = { currentFrame: 0 };
		// Evaluate the emitted source with the same globals supplied by AudioWorkletGlobalScope.
		const factory = new Function(
			"AudioWorkletProcessor",
			"registerProcessor",
			"sampleRate",
			"clock",
			cameraAudioWorkletSource().replaceAll(
				"currentFrame",
				"clock.currentFrame",
			),
		);
		factory(
			BaseProcessor,
			(_name: string, value: typeof Processor) => {
				Processor = value;
			},
			16_000,
			clock,
		);
		const processor = new Processor({ processorOptions: { bufferSeconds: 1 } });
		const output = [new Float32Array(1600)];
		processor.process([[new Float32Array(1600).fill(0.5)]], [output]);
		processor.port.onmessage({
			data: { type: "snapshot", id: 1, durationMs: 1000, cutoff: 0.1 },
		});
		expect(messages.at(-1)?.pcm.length).toBe(1600);
		expect(output[0].every((sample) => sample === 0)).toBe(true);
		processor.port.onmessage({ data: { type: "pause", epoch: 1 } });
		clock.currentFrame = 1600;
		processor.process([[new Float32Array(1600).fill(0.9)]], [output]);
		processor.port.onmessage({ data: { type: "resume", epoch: 2 } });
		processor.port.onmessage({
			data: { type: "snapshot", id: 2, durationMs: 1000, cutoff: 0.2 },
		});
		expect(messages.at(-1)?.pcm.length).toBe(0);
		clock.currentFrame = 3200;
		processor.process([[new Float32Array(1600).fill(-0.25)]], [output]);
		processor.port.onmessage({
			data: { type: "snapshot", id: 3, durationMs: 1000, cutoff: 0.3 },
		});
		expect(messages.at(-1)?.pcm.length).toBe(1600);
		expect(messages.at(-1)?.pcm[0]).toBe(-0.25);
		expect(messages.at(-1)?.epoch).toBe(2);
	});
});
