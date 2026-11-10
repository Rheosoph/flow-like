export const TURN_SAMPLE_RATE = 16_000;
export const TURN_WINDOW_SAMPLES = 8 * TURN_SAMPLE_RATE;
export const TURN_MEL_BINS = 80;
export const TURN_MEL_FRAMES = 800;

export const VAD_FRAME_SAMPLES = 512;
export const VAD_CONTEXT_SAMPLES = 64;

const N_FFT = 400;
const HOP = 160;
const FFT_BINS = N_FFT / 2 + 1;
const MEL_FLOOR = 1e-10;
const NORM_VARIANCE_EPS = 1e-7;

/** The last 8 s of `samples`, zero-padded at the front when shorter. */
export function lastTurnWindow(samples: Float32Array): Float32Array {
	if (samples.length >= TURN_WINDOW_SAMPLES) {
		return samples.subarray(samples.length - TURN_WINDOW_SAMPLES);
	}
	const out = new Float32Array(TURN_WINDOW_SAMPLES);
	out.set(samples, TURN_WINDOW_SAMPLES - samples.length);
	return out;
}

const hertzToMel = (hz: number) =>
	hz >= 1000 ? 15 + Math.log(hz / 1000) * (27 / Math.log(6.4)) : (3 * hz) / 200;

const melToHertz = (mel: number) =>
	mel >= 15
		? 1000 * Math.exp((Math.log(6.4) / 27) * (mel - 15))
		: (200 * mel) / 3;

interface MelBand {
	start: number;
	weights: Float64Array;
}

function buildMelBands(): MelBand[] {
	const melMax = hertzToMel(TURN_SAMPLE_RATE / 2);
	const filterFreqs = Array.from({ length: TURN_MEL_BINS + 2 }, (_, i) =>
		melToHertz((melMax * i) / (TURN_MEL_BINS + 1)),
	);
	const fftFreqs = Array.from(
		{ length: FFT_BINS },
		(_, i) => (i * (TURN_SAMPLE_RATE / 2)) / (FFT_BINS - 1),
	);
	const bands: MelBand[] = [];
	for (let m = 0; m < TURN_MEL_BINS; m++) {
		const [low, mid, high] = [
			filterFreqs[m],
			filterFreqs[m + 1],
			filterFreqs[m + 2],
		];
		const norm = 2 / (high - low);
		const weights = fftFreqs.map(
			(freq) =>
				Math.max(
					0,
					Math.min((freq - low) / (mid - low), (high - freq) / (high - mid)),
				) * norm,
		);
		const start = weights.findIndex((w) => w > 0);
		const end = weights.findLastIndex((w) => w > 0);
		bands.push({
			start: Math.max(0, start),
			weights: Float64Array.from(weights.slice(Math.max(0, start), end + 1)),
		});
	}
	return bands;
}

const HANN = Float64Array.from(
	{ length: N_FFT },
	(_, n) => 0.5 - 0.5 * Math.cos((2 * Math.PI * n) / N_FFT),
);
const TWIDDLE_COS = Float64Array.from({ length: N_FFT }, (_, j) =>
	Math.cos((2 * Math.PI * j) / N_FFT),
);
const TWIDDLE_SIN = Float64Array.from({ length: N_FFT }, (_, j) =>
	Math.sin((2 * Math.PI * j) / N_FFT),
);
const RADIX_RE = new Float64Array(N_FFT);
const RADIX_IM = new Float64Array(N_FFT);
let melBands: MelBand[] | undefined;

function smallestFactor(n: number) {
	for (let p = 2; p * p <= n; p++) if (n % p === 0) return p;
	return n;
}

/** Turns the `p` interleaved `m`-point transforms at `outOffset` into one `p * m`-point transform. */
function combine(
	outRe: Float64Array,
	outIm: Float64Array,
	outOffset: number,
	p: number,
	m: number,
) {
	const n = p * m;
	const scale = N_FFT / n;
	for (let k = 0; k < m; k++) {
		for (let q = 0; q < p; q++) {
			const bin = k + q * m;
			let re = 0;
			let im = 0;
			for (let r = 0; r < p; r++) {
				const index = (((r * bin) % n) * scale) % N_FFT;
				const c = TWIDDLE_COS[index];
				const s = -TWIDDLE_SIN[index];
				const yr = outRe[outOffset + r * m + k];
				const yi = outIm[outOffset + r * m + k];
				re += yr * c - yi * s;
				im += yr * s + yi * c;
			}
			RADIX_RE[q] = re;
			RADIX_IM[q] = im;
		}
		for (let q = 0; q < p; q++) {
			outRe[outOffset + k + q * m] = RADIX_RE[q];
			outIm[outOffset + k + q * m] = RADIX_IM[q];
		}
	}
}

/** Mixed-radix decimation-in-time FFT of one `n`-point strided input (n divides N_FFT). */
function fft(
	inRe: Float64Array,
	inIm: Float64Array,
	offset: number,
	stride: number,
	n: number,
	outRe: Float64Array,
	outIm: Float64Array,
	outOffset: number,
) {
	if (n === 1) {
		outRe[outOffset] = inRe[offset];
		outIm[outOffset] = inIm[offset];
		return;
	}
	const p = smallestFactor(n);
	const m = n / p;
	for (let r = 0; r < p; r++) {
		fft(
			inRe,
			inIm,
			offset + r * stride,
			stride * p,
			m,
			outRe,
			outIm,
			outOffset + r * m,
		);
	}
	combine(outRe, outIm, outOffset, p, m);
}

function normalize(window: Float32Array) {
	const n = window.length;
	let mean = 0;
	for (let i = 0; i < n; i++) mean += window[i];
	mean /= n;
	let variance = 0;
	for (let i = 0; i < n; i++) variance += (window[i] - mean) ** 2;
	variance /= n;
	const scale = 1 / Math.sqrt(variance + NORM_VARIANCE_EPS);
	return Float64Array.from(window, (v) => (v - mean) * scale);
}

function reflectPad(x: Float64Array, pad: number) {
	const out = new Float64Array(x.length + 2 * pad);
	out.set(x, pad);
	for (let i = 0; i < pad; i++) {
		out[i] = x[pad - i];
		out[pad + x.length + i] = x[x.length - 2 - i];
	}
	return out;
}

/**
 * Whisper-style log-mel features of exactly 8 s of 16 kHz audio, row-major `[80][800]`;
 * the same maths as the Smart Turn reference (waveform normalized, trailing frame dropped).
 */
export function turnLogMel(window: Float32Array): Float32Array {
	if (window.length !== TURN_WINDOW_SAMPLES) {
		throw new Error(`Turn window must be ${TURN_WINDOW_SAMPLES} samples`);
	}
	melBands ??= buildMelBands();
	const padded = reflectPad(normalize(window), N_FFT / 2);
	const frameRe = new Float64Array(N_FFT);
	const frameIm = new Float64Array(N_FFT);
	const outRe = new Float64Array(N_FFT);
	const outIm = new Float64Array(N_FFT);
	const powerA = new Float64Array(FFT_BINS);
	const powerB = new Float64Array(FFT_BINS);
	const logMel = new Float64Array(TURN_MEL_BINS * TURN_MEL_FRAMES);
	let top = Number.NEGATIVE_INFINITY;
	const writeFrame = (power: Float64Array, t: number) => {
		for (let m = 0; m < TURN_MEL_BINS; m++) {
			const { start, weights } = (melBands as MelBand[])[m];
			let sum = 0;
			for (let j = 0; j < weights.length; j++)
				sum += weights[j] * power[start + j];
			const value = Math.log10(Math.max(MEL_FLOOR, sum));
			logMel[m * TURN_MEL_FRAMES + t] = value;
			if (value > top) top = value;
		}
	};
	// Two real frames ride one complex transform: frame t as the real part, t + 1 as the imaginary.
	for (let t = 0; t < TURN_MEL_FRAMES; t += 2) {
		for (let i = 0; i < N_FFT; i++) {
			frameRe[i] = padded[t * HOP + i] * HANN[i];
			frameIm[i] = padded[(t + 1) * HOP + i] * HANN[i];
		}
		fft(frameRe, frameIm, 0, 1, N_FFT, outRe, outIm, 0);
		for (let b = 0; b < FFT_BINS; b++) {
			const mirror = (N_FFT - b) % N_FFT;
			const ar = (outRe[b] + outRe[mirror]) / 2;
			const ai = (outIm[b] - outIm[mirror]) / 2;
			const br = (outIm[b] + outIm[mirror]) / 2;
			const bi = (outRe[mirror] - outRe[b]) / 2;
			powerA[b] = ar * ar + ai * ai;
			powerB[b] = br * br + bi * bi;
		}
		writeFrame(powerA, t);
		writeFrame(powerB, t + 1);
	}
	const floor = top - 8;
	return Float32Array.from(logMel, (v) => (Math.max(v, floor) + 4) / 4);
}

/**
 * Cuts a 16 kHz stream into Silero frames of 512 samples, each prefixed with the previous
 * frame's last 64 samples, as the reference `OnnxWrapper` does.
 */
export function createVadFramer() {
	let pending = new Float32Array(0);
	let context = new Float32Array(VAD_CONTEXT_SAMPLES);
	return {
		push(samples: Float32Array): Float32Array[] {
			const joined = new Float32Array(pending.length + samples.length);
			joined.set(pending);
			joined.set(samples, pending.length);
			const frames: Float32Array[] = [];
			let at = 0;
			while (joined.length - at >= VAD_FRAME_SAMPLES) {
				const frame = joined.subarray(at, at + VAD_FRAME_SAMPLES);
				const input = new Float32Array(VAD_CONTEXT_SAMPLES + VAD_FRAME_SAMPLES);
				input.set(context);
				input.set(frame, VAD_CONTEXT_SAMPLES);
				context = frame.slice(VAD_FRAME_SAMPLES - VAD_CONTEXT_SAMPLES);
				frames.push(input);
				at += VAD_FRAME_SAMPLES;
			}
			pending = joined.slice(at);
			return frames;
		},
		reset() {
			pending = new Float32Array(0);
			context = new Float32Array(VAD_CONTEXT_SAMPLES);
		},
	};
}
