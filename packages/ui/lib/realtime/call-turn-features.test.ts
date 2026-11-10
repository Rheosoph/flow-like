import { describe, expect, test } from "bun:test";
import {
	TURN_MEL_BINS,
	TURN_MEL_FRAMES,
	TURN_WINDOW_SAMPLES,
	VAD_CONTEXT_SAMPLES,
	VAD_FRAME_SAMPLES,
	createVadFramer,
	lastTurnWindow,
	turnLogMel,
} from "./call-turn-features";

const N_FFT = 400;
const HOP = 160;

function referenceLogMel(window: Float32Array): Float32Array {
	const n = window.length;
	const mean = window.reduce((sum, v) => sum + v, 0) / n;
	const variance = window.reduce((sum, v) => sum + (v - mean) ** 2, 0) / n;
	const x = Array.from(window, (v) => (v - mean) / Math.sqrt(variance + 1e-7));
	const pad = N_FFT / 2;
	const padded = [
		...Array.from({ length: pad }, (_, i) => x[pad - i]),
		...x,
		...Array.from({ length: pad }, (_, i) => x[n - 2 - i]),
	];
	const hertzToMel = (hz: number) =>
		hz >= 1000
			? 15 + Math.log(hz / 1000) * (27 / Math.log(6.4))
			: (3 * hz) / 200;
	const melToHertz = (mel: number) =>
		mel >= 15
			? 1000 * Math.exp((Math.log(6.4) / 27) * (mel - 15))
			: (200 * mel) / 3;
	const edges = Array.from({ length: TURN_MEL_BINS + 2 }, (_, i) =>
		melToHertz((hertzToMel(8000) * i) / (TURN_MEL_BINS + 1)),
	);
	const filters = Array.from({ length: TURN_MEL_BINS }, (_, m) =>
		Array.from({ length: N_FFT / 2 + 1 }, (_, k) => {
			const freq = (k * 8000) / (N_FFT / 2);
			const up = (freq - edges[m]) / (edges[m + 1] - edges[m]);
			const down = (edges[m + 2] - freq) / (edges[m + 2] - edges[m + 1]);
			return (Math.max(0, Math.min(up, down)) * 2) / (edges[m + 2] - edges[m]);
		}),
	);
	const hann = Array.from(
		{ length: N_FFT },
		(_, i) => 0.5 - 0.5 * Math.cos((2 * Math.PI * i) / N_FFT),
	);
	const cos = Array.from({ length: N_FFT }, (_, j) =>
		Math.cos((2 * Math.PI * j) / N_FFT),
	);
	const sin = Array.from({ length: N_FFT }, (_, j) =>
		Math.sin((2 * Math.PI * j) / N_FFT),
	);
	const out = new Float64Array(TURN_MEL_BINS * TURN_MEL_FRAMES);
	let top = Number.NEGATIVE_INFINITY;
	for (let t = 0; t < TURN_MEL_FRAMES; t++) {
		const power: number[] = [];
		for (let k = 0; k <= N_FFT / 2; k++) {
			let re = 0;
			let im = 0;
			for (let i = 0; i < N_FFT; i++) {
				const weighted = padded[t * HOP + i] * hann[i];
				re += weighted * cos[(k * i) % N_FFT];
				im -= weighted * sin[(k * i) % N_FFT];
			}
			power.push(re * re + im * im);
		}
		for (let m = 0; m < TURN_MEL_BINS; m++) {
			const sum = filters[m].reduce((acc, w, k) => acc + w * power[k], 0);
			const value = Math.log10(Math.max(1e-10, sum));
			out[m * TURN_MEL_FRAMES + t] = value;
			top = Math.max(top, value);
		}
	}
	return Float32Array.from(out, (v) => (Math.max(v, top - 8) + 4) / 4);
}

describe("lastTurnWindow", () => {
	test("keeps the last 8 s of a longer recording", () => {
		const samples = Float32Array.from(
			{ length: TURN_WINDOW_SAMPLES + 10 },
			(_, i) => i,
		);
		const window = lastTurnWindow(samples);
		expect(window.length).toBe(TURN_WINDOW_SAMPLES);
		expect(window[0]).toBe(10);
		expect(window[TURN_WINDOW_SAMPLES - 1]).toBe(TURN_WINDOW_SAMPLES + 9);
	});

	test("pads a shorter recording with zeros at the front", () => {
		const window = lastTurnWindow(Float32Array.from([1, 2, 3]));
		expect(window.length).toBe(TURN_WINDOW_SAMPLES);
		expect(Array.from(window.subarray(TURN_WINDOW_SAMPLES - 4))).toEqual([
			0, 1, 2, 3,
		]);
		expect(window[0]).toBe(0);
	});
});

describe("turnLogMel", () => {
	test("rejects a window of the wrong length", () => {
		expect(() => turnLogMel(new Float32Array(100))).toThrow();
	});

	test("matches a direct DFT implementation of the reference maths", () => {
		const speech = Float32Array.from(
			{ length: 3 * 16_000 },
			(_, i) =>
				0.3 * Math.sin(i / 7) * Math.sin(i / 4000) +
				0.1 * Math.sin(i / 2.3) +
				((i * 7919) % 101) / 5000,
		);
		const window = lastTurnWindow(speech);
		const fast = turnLogMel(window);
		const slow = referenceLogMel(window);
		expect(fast.length).toBe(TURN_MEL_BINS * TURN_MEL_FRAMES);
		let worst = 0;
		for (let i = 0; i < fast.length; i++) {
			worst = Math.max(worst, Math.abs(fast[i] - slow[i]));
		}
		expect(worst).toBeLessThan(1e-4);
	}, 120_000);

	test("the floor sits 8 log10 units under the peak", () => {
		const features = turnLogMel(
			lastTurnWindow(
				Float32Array.from({ length: 8000 }, (_, i) => Math.sin(i)),
			),
		);
		let top = Number.NEGATIVE_INFINITY;
		let bottom = Number.POSITIVE_INFINITY;
		for (const value of features) {
			top = Math.max(top, value);
			bottom = Math.min(bottom, value);
		}
		expect(top - bottom).toBeCloseTo(2, 5);
	});
});

describe("createVadFramer", () => {
	test("emits 512-sample frames with the previous frame's 64-sample tail as context", () => {
		const framer = createVadFramer();
		const samples = Float32Array.from({ length: 1100 }, (_, i) => i + 1);
		const frames = framer.push(samples);
		expect(frames).toHaveLength(2);
		expect(frames[0].length).toBe(VAD_CONTEXT_SAMPLES + VAD_FRAME_SAMPLES);
		expect(frames[0][VAD_CONTEXT_SAMPLES - 1]).toBe(0);
		expect(frames[0][VAD_CONTEXT_SAMPLES]).toBe(1);
		expect(frames[1][0]).toBe(VAD_FRAME_SAMPLES - VAD_CONTEXT_SAMPLES + 1);
		expect(frames[1][VAD_CONTEXT_SAMPLES]).toBe(VAD_FRAME_SAMPLES + 1);
	});

	test("carries a partial frame over to the next push and resets", () => {
		const framer = createVadFramer();
		expect(framer.push(new Float32Array(300))).toHaveLength(0);
		expect(framer.push(new Float32Array(300))).toHaveLength(1);
		framer.reset();
		expect(framer.push(new Float32Array(300))).toHaveLength(0);
	});
});
