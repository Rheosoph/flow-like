import { describe, expect, test } from "bun:test";
import { type IBit, IBitTypes } from "../schema";
import {
	STT_LOCAL_PROVIDER,
	bytesToBase64,
	createSerialQueue,
	createUtteranceSegmenter,
	encodeWav,
	resample,
	resolveSttChoice,
} from "./call-listening";

const RATE = 16_000;
const CHUNK = 1600;

const tone = (ms: number, amplitude = 0.3) =>
	Float32Array.from(
		{ length: (RATE * ms) / 1000 },
		(_, i) => Math.sin(i / 5) * amplitude,
	);
const silence = (ms: number) => new Float32Array((RATE * ms) / 1000);

function feed(
	segmenter: ReturnType<typeof createUtteranceSegmenter>,
	signal: Float32Array,
) {
	const out = [];
	for (let at = 0; at < signal.length; at += CHUNK) {
		out.push(...segmenter.push(signal.subarray(at, at + CHUNK)));
	}
	return out;
}

const join = (...parts: Float32Array[]) => {
	const out = new Float32Array(parts.reduce((n, p) => n + p.length, 0));
	let at = 0;
	for (const part of parts) {
		out.set(part, at);
		at += part.length;
	}
	return out;
};

describe("createUtteranceSegmenter", () => {
	test("cuts one utterance after 700 ms of silence", () => {
		const segmenter = createUtteranceSegmenter();
		const found = feed(
			segmenter,
			join(silence(500), tone(1000), silence(1000)),
		);
		expect(found).toHaveLength(1);
		expect(found[0].offsetMs).toBeGreaterThanOrEqual(300);
		expect(found[0].offsetMs).toBeLessThanOrEqual(500);
		expect(found[0].durationMs).toBeGreaterThanOrEqual(1000);
		expect(found[0].durationMs).toBeLessThan(1500);
	});

	test("waits for the full silence before ending", () => {
		const segmenter = createUtteranceSegmenter();
		expect(feed(segmenter, join(tone(1000), silence(400)))).toHaveLength(0);
		expect(segmenter.flush()?.durationMs).toBeGreaterThan(1000);
	});

	test("joins speech separated by a short pause", () => {
		const segmenter = createUtteranceSegmenter();
		const found = feed(
			segmenter,
			join(tone(800), silence(400), tone(800), silence(1000)),
		);
		expect(found).toHaveLength(1);
	});

	test("splits on a long pause", () => {
		const segmenter = createUtteranceSegmenter();
		const found = feed(
			segmenter,
			join(tone(800), silence(1000), tone(800), silence(1000)),
		);
		expect(found).toHaveLength(2);
		expect(found[1].offsetMs).toBeGreaterThan(found[0].offsetMs + 1500);
	});

	test("drops a blip shorter than 600 ms", () => {
		const segmenter = createUtteranceSegmenter();
		expect(feed(segmenter, join(tone(200), silence(1500)))).toHaveLength(0);
		expect(segmenter.flush()).toBeNull();
	});

	test("ignores steady silence and quiet noise", () => {
		const segmenter = createUtteranceSegmenter();
		expect(
			feed(segmenter, join(silence(2000), tone(2000, 0.002))),
		).toHaveLength(0);
	});

	test("cuts at 15 s of continuous speech", () => {
		const segmenter = createUtteranceSegmenter();
		const found = feed(segmenter, tone(31_000));
		expect(found.length).toBeGreaterThanOrEqual(2);
		for (const utterance of found) {
			expect(utterance.durationMs).toBeLessThanOrEqual(15_200);
		}
	});

	test("reset drops a half-heard utterance", () => {
		const segmenter = createUtteranceSegmenter();
		feed(segmenter, tone(1000));
		segmenter.reset();
		expect(segmenter.flush()).toBeNull();
	});
});

describe("resample", () => {
	test("returns the input at the same rate", () => {
		const input = tone(100);
		expect(resample(input, RATE)).toBe(input);
	});

	test("downsamples 48 kHz to 16 kHz by a third", () => {
		const input = new Float32Array(4800).fill(0.5);
		const out = resample(input, 48_000);
		expect(out.length).toBe(1600);
		expect(out[10]).toBeCloseTo(0.5);
	});
});

describe("encodeWav", () => {
	test("writes a 16-bit mono PCM header and samples", () => {
		const wav = encodeWav(Float32Array.from([0, 1, -1, 0.5]));
		const view = new DataView(wav.buffer);
		const tag = (at: number) =>
			String.fromCharCode(...wav.subarray(at, at + 4));
		expect(tag(0)).toBe("RIFF");
		expect(tag(8)).toBe("WAVE");
		expect(tag(12)).toBe("fmt ");
		expect(tag(36)).toBe("data");
		expect(view.getUint32(4, true)).toBe(36 + 8);
		expect(view.getUint16(20, true)).toBe(1);
		expect(view.getUint16(22, true)).toBe(1);
		expect(view.getUint32(24, true)).toBe(16_000);
		expect(view.getUint32(28, true)).toBe(32_000);
		expect(view.getUint16(34, true)).toBe(16);
		expect(view.getUint32(40, true)).toBe(8);
		expect(wav.length).toBe(52);
		expect(view.getInt16(46, true)).toBe(32767);
		expect(view.getInt16(48, true)).toBe(-32768);
	});

	test("base64 round-trips the bytes", () => {
		const wav = encodeWav(tone(100));
		expect(
			Uint8Array.from(atob(bytesToBase64(wav)), (c) => c.charCodeAt(0)),
		).toEqual(wav);
	});
});

const bit = (id: string, provider: string, type = IBitTypes.Stt): IBit =>
	({
		id,
		hub: "hub.example",
		type,
		parameters: { provider: { provider_name: provider } },
	}) as unknown as IBit;

describe("resolveSttChoice", () => {
	const local = bit("whisper", STT_LOCAL_PROVIDER);
	const hosted = bit("or-whisper", "hosted:openrouter");
	const none = new Set<string>();

	test("desktop prefers an installed local bit", () => {
		expect(
			resolveSttChoice([hosted, local], {
				desktop: true,
				installed: new Set(["whisper"]),
			}),
		).toEqual({ bitId: "hub.example:whisper", route: "local" });
	});

	test("desktop falls back to a remote bit when the local one is not installed", () => {
		expect(
			resolveSttChoice([local, hosted], { desktop: true, installed: none }),
		).toEqual({ bitId: "or-whisper", route: "remote" });
	});

	test("the web never picks a local bit", () => {
		expect(
			resolveSttChoice([local, hosted], {
				desktop: false,
				installed: new Set(["whisper"]),
			}),
		).toEqual({ bitId: "or-whisper", route: "remote" });
	});

	test("nothing usable gives null", () => {
		expect(
			resolveSttChoice([local, bit("llm", "openai", IBitTypes.Llm)], {
				desktop: false,
				installed: none,
			}),
		).toBeNull();
		expect(resolveSttChoice([], { desktop: true, installed: none })).toBeNull();
	});
});

describe("createSerialQueue", () => {
	test("runs one at a time and drops the oldest beyond the limit", async () => {
		const ran: number[] = [];
		let release: () => void = () => {};
		let active = 0;
		let overlap = 0;
		const queue = createSerialQueue<number>(async (item) => {
			active++;
			overlap = Math.max(overlap, active);
			ran.push(item);
			if (item === 1) {
				await new Promise<void>((r) => {
					release = r;
				});
			}
			active--;
		}, 3);
		for (const item of [1, 2, 3, 4, 5, 6]) queue.push(item);
		release();
		await new Promise((r) => setTimeout(r, 10));
		expect(ran).toEqual([1, 4, 5, 6]);
		expect(overlap).toBe(1);
	});

	test("close stops further work", async () => {
		const ran: number[] = [];
		const queue = createSerialQueue<number>(async (item) => {
			ran.push(item);
		});
		queue.close();
		queue.push(1);
		await new Promise((r) => setTimeout(r, 5));
		expect(ran).toEqual([]);
	});
});
