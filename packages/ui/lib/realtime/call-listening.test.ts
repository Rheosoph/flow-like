import { describe, expect, test } from "bun:test";
import { type IBit, IBitTypes } from "../schema";
import {
	BARGE_IN_CONFIRM_MS,
	BARGE_IN_VAD_PROBABILITY,
	END_SILENCE_MS,
	INCOMPLETE_TURN_SILENCE_MS,
	MIN_SPEECH_MS,
	STT_LOCAL_PROVIDER,
	TURN_CHECK_SILENCE_MS,
	type Utterance,
	VAD_CONTINUE_PROBABILITY,
	VAD_START_PROBABILITY,
	bytesToBase64,
	createBargeInDetector,
	createSerialQueue,
	createUtteranceSegmenter,
	encodeWav,
	resample,
	resolveSttChoice,
	rms,
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
	test("previews growing audio during speech and preserves the final utterance", () => {
		const previews: Utterance[] = [];
		const segmenter = createUtteranceSegmenter({
			onPartial: (utterance) => previews.push(utterance),
		});
		expect(feed(segmenter, tone(2500))).toEqual([]);
		expect(previews.map((utterance) => utterance.durationMs)).toEqual([
			1000, 2000,
		]);
		expect(previews.map((utterance) => utterance.offsetMs)).toEqual([0, 0]);
		const firstSamples = previews[0].samples.slice();
		const final = feed(segmenter, silence(1000));
		expect(final).toHaveLength(1);
		expect(final[0].durationMs).toBe(2650);
		expect(final[0].samples.slice(0, firstSamples.length)).toEqual(
			firstSamples,
		);
		expect(previews[0].samples).toEqual(firstSamples);
		expect(previews).toHaveLength(3);
		expect(previews[2].settled).toBe(true);
		expect(previews[2].samples).toEqual(final[0].samples);
		expect(previews[2].offsetMs).toBe(final[0].offsetMs);
		expect(previews[2].durationMs).toBe(final[0].durationMs);
		expect(segmenter.flush()).toBeNull();
	});

	test("previews respect minimum speech and their configured interval", () => {
		const previews: Utterance[] = [];
		const segmenter = createUtteranceSegmenter({
			partialMs: 300,
			onPartial: (utterance) => previews.push(utterance),
		});
		feed(segmenter, join(tone(200), silence(1000)));
		expect(previews).toEqual([]);
		feed(segmenter, tone(1000));
		expect(previews.map((utterance) => utterance.durationMs)).toEqual([
			600, 900, 1200,
		]);
	});

	test("reset starts previews afresh without including discarded speech", () => {
		const previews: Utterance[] = [];
		const segmenter = createUtteranceSegmenter({
			onPartial: (utterance) => previews.push(utterance),
		});
		feed(segmenter, tone(1500));
		segmenter.reset();
		feed(segmenter, tone(1000, 0.1));
		expect(previews.map((utterance) => utterance.durationMs)).toEqual([
			1000, 1000,
		]);
		expect(previews.map((utterance) => utterance.offsetMs)).toEqual([0, 1500]);
		expect(previews[1].samples).toEqual(tone(1000, 0.1));
		expect(segmenter.flush()?.samples).toEqual(previews[1].samples);
	});

	test("maximum-length cuts restart previews without duplicating the final audio", () => {
		const previews: Utterance[] = [];
		const segmenter = createUtteranceSegmenter({
			maxMs: 2000,
			onPartial: (utterance) => previews.push(utterance),
		});
		const final = feed(segmenter, tone(4000));
		expect(final.map((utterance) => utterance.offsetMs)).toEqual([0, 2000]);
		expect(final.map((utterance) => utterance.durationMs)).toEqual([
			2000, 2000,
		]);
		expect(previews.map((utterance) => utterance.offsetMs)).toEqual([0, 2000]);
		expect(previews.map((utterance) => utterance.durationMs)).toEqual([
			1000, 1000,
		]);
	});

	test("cuts one utterance after 450 ms of silence", () => {
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

	test("a settled preview is emitted once, with exactly the final's audio", () => {
		const previews: Utterance[] = [];
		const segmenter = createUtteranceSegmenter({
			onPartial: (utterance) => previews.push(utterance),
		});
		const found = feed(segmenter, join(tone(800), silence(1000)));
		const settled = previews.filter((utterance) => utterance.settled);
		expect(END_SILENCE_MS).toBe(450);
		expect(settled).toHaveLength(1);
		expect(found).toHaveLength(1);
		expect(settled[0].offsetMs).toBe(found[0].offsetMs);
		expect(settled[0].samples).toEqual(found[0].samples);
		expect(settled[0].durationMs).toBe(found[0].durationMs);
	});

	test("speech resuming after a settled preview settles again with the longer audio", () => {
		const previews: Utterance[] = [];
		const segmenter = createUtteranceSegmenter({
			onPartial: (utterance) => previews.push(utterance),
		});
		const found = feed(
			segmenter,
			join(tone(800), silence(300), tone(800), silence(1000)),
		);
		const settled = previews.filter((utterance) => utterance.settled);
		expect(settled).toHaveLength(2);
		expect(settled[1].samples.length).toBeGreaterThan(
			settled[0].samples.length,
		);
		expect(settled[1].samples).toEqual(found[0].samples);
	});

	test("short noises never settle", () => {
		const previews: Utterance[] = [];
		const segmenter = createUtteranceSegmenter({
			onPartial: (utterance) => previews.push(utterance),
		});
		feed(segmenter, join(tone(200), silence(1000)));
		expect(previews).toEqual([]);
	});

	test("waits for the full silence before ending", () => {
		const segmenter = createUtteranceSegmenter();
		expect(feed(segmenter, join(tone(1000), silence(300)))).toHaveLength(0);
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

	test("skips custom keys, legacy aliases and unsupported hosted providers", () => {
		const unsupported = [
			"custom:openai",
			"Hosted",
			"Premium",
			"hosted:anthropic",
			"",
		].map((provider, index) => bit(`unsupported-${index}`, provider));
		for (const desktop of [true, false]) {
			expect(
				resolveSttChoice(unsupported, { desktop, installed: none }),
			).toBeNull();
			expect(
				resolveSttChoice([...unsupported, hosted], {
					desktop,
					installed: none,
				}),
			).toEqual({ bitId: hosted.id, route: "remote" });
		}
	});

	test("accepts each hosted speech provider with normalized names", () => {
		for (const provider of [
			"openrouter",
			"openai",
			"groq",
			"mistral",
			"together",
			"xai",
			"gemini",
		]) {
			expect(
				resolveSttChoice(
					[bit("speech", ` HOSTED:${provider.toUpperCase()} `)],
					{
						desktop: false,
						installed: none,
					},
				),
			).toEqual({ bitId: "speech", route: "remote" });
		}
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

describe("createBargeInDetector", () => {
	const detect = (signal: Float32Array) => {
		const detector = createBargeInDetector();
		const hits: Float32Array[][] = [];
		for (let at = 0; at < signal.length; at += CHUNK) {
			const hit = detector.push(signal.subarray(at, at + CHUNK));
			if (hit) hits.push(hit);
		}
		return hits;
	};

	test("ignores speech-level and echo-level audio below the raised threshold", () => {
		expect(detect(tone(3000, 0.04))).toEqual([]);
	});

	test("needs 300 ms of loud audio and tolerates short gaps", () => {
		expect(detect(tone(200, 0.4))).toEqual([]);
		expect(
			detect(join(tone(200, 0.4), silence(100), tone(100, 0.4))),
		).toHaveLength(1);
		expect(
			detect(join(tone(100, 0.4), silence(900), tone(100, 0.4), silence(900))),
		).toEqual([]);
	});

	test("returns the window starting just before the first loud chunk", () => {
		const [hit] = detect(join(silence(300), tone(400, 0.4)));
		expect(hit.length).toBeLessThanOrEqual(6);
		expect(hit.length).toBeGreaterThanOrEqual(4);
		expect(hit[0].every((value) => value === 0)).toBe(true);
	});
});

describe("createUtteranceSegmenter with a voice-activity model", () => {
	const speechOf = (chunk: Float32Array) => (rms(chunk) > 0.01 ? 0.9 : 0.02);

	function harness(
		options: { turnModel?: boolean; onPartial?: (u: Utterance) => void } = {},
	) {
		const asks: { id: number; samples: Float32Array }[] = [];
		const segmenter = createUtteranceSegmenter({
			onPartial: options.onPartial,
			onTurnCheck:
				options.turnModel === false
					? undefined
					: (id, samples) => {
							asks.push({ id, samples });
							return true;
						},
		});
		const push = (signal: Float32Array, probability = speechOf) => {
			const out: Utterance[] = [];
			for (let at = 0; at < signal.length; at += CHUNK) {
				const chunk = signal.subarray(at, at + CHUNK);
				out.push(...segmenter.push(chunk, probability(chunk)));
			}
			return out;
		};
		return { segmenter, asks, push };
	}

	test("a short 'Ja' passes with the model and is dropped by the energy rule", () => {
		const h = harness();
		expect(h.push(join(silence(300), tone(300, 0.3), silence(200)))).toEqual(
			[],
		);
		expect(h.asks).toHaveLength(1);
		const [done] = h.segmenter.answerTurn(h.asks[0].id, 0.95);
		expect(done.turn).toMatchObject({ source: "model", probability: 0.95 });

		const energy = createUtteranceSegmenter();
		expect(
			feed(energy, join(silence(300), tone(300, 0.3), silence(600))),
		).toEqual([]);
	});

	test("rejects a sound shorter than the minimum speech", () => {
		const h = harness();
		h.push(join(tone(100, 0.3), silence(300)));
		expect(h.segmenter.answerTurn(h.asks[0].id, 0.95)).toEqual([]);
	});

	test("a short answer still passes when its last blocks were judged by energy", () => {
		const h = harness({ turnModel: false });
		expect(h.push(join(silence(300), tone(300, 0.3)))).toEqual([]);
		const out = feed(h.segmenter, silence(500));
		expect(out).toHaveLength(1);
		expect(out[0].turn?.source).toBe("silence");
	});

	test("an incomplete verdict keeps holding through blocks judged by energy", () => {
		const h = harness();
		h.push(join(tone(1000, 0.3), silence(200)));
		expect(h.segmenter.answerTurn(h.asks[0].id, 0.1)).toEqual([]);
		expect(feed(h.segmenter, silence(1200))).toEqual([]);
		const [done] = feed(h.segmenter, silence(100));
		expect(done.turn).toMatchObject({ source: "silence", waitedMs: 1500 });
	});

	test("hysteresis starts above 0.5 and continues above 0.35", () => {
		const segmenter = createUtteranceSegmenter({ onTurnCheck: () => true });
		const chunk = tone(100, 0.3);
		segmenter.push(chunk, 0.4);
		expect(segmenter.flush()).toBeNull();
		segmenter.push(chunk, 0.6);
		segmenter.push(chunk, 0.4);
		segmenter.push(chunk, 0.4);
		segmenter.push(chunk, 0.2);
		const done = segmenter.flush();
		expect(done?.turn?.waitedMs).toBe(100);
		expect(done?.durationMs).toBeGreaterThanOrEqual(300);
	});

	test("a complete turn finishes as soon as the model says so, about 200 ms after speech", () => {
		const previews: Utterance[] = [];
		const h = harness({ onPartial: (u) => previews.push(u) });
		const speech = join(silence(200), tone(1000, 0.3));
		expect(h.push(speech)).toEqual([]);
		expect(h.asks).toHaveLength(0);
		expect(h.push(silence(100))).toEqual([]);
		expect(h.asks).toHaveLength(0);
		expect(h.push(silence(100))).toEqual([]);
		expect(h.asks).toHaveLength(1);
		const settled = previews.filter((u) => u.settled);
		expect(settled).toHaveLength(1);
		const [done] = h.segmenter.answerTurn(h.asks[0].id, 0.8);
		expect(done.turn).toEqual({
			source: "model",
			probability: 0.8,
			waitedMs: 200,
		});
		expect(done.speechEndMs).toBe(1200);
		expect(done.samples).toEqual(settled[0].samples);
		expect(done.offsetMs).toBe(settled[0].offsetMs);
		expect(done.durationMs).toBe(settled[0].durationMs);
	});

	test("the turn check sees the whole utterance so far", () => {
		const h = harness();
		h.push(join(tone(1000, 0.3), silence(200)));
		expect(h.asks[0].samples.length).toBe(RATE * 1.2);
	});

	test("an incomplete turn keeps listening, asks again after speech resumes and ends at 1500 ms", () => {
		const h = harness();
		h.push(join(tone(1000, 0.3), silence(200)));
		expect(h.segmenter.answerTurn(h.asks[0].id, 0.1)).toEqual([]);
		expect(h.push(silence(1000))).toEqual([]);
		h.push(tone(500, 0.3));
		expect(h.asks).toHaveLength(1);
		expect(h.push(silence(200))).toEqual([]);
		expect(h.asks).toHaveLength(2);
		expect(h.segmenter.answerTurn(h.asks[1].id, 0.2)).toEqual([]);
		expect(h.push(silence(1200))).toEqual([]);
		const [done] = h.push(silence(100));
		expect(done.turn).toEqual({
			source: "silence",
			probability: 0.2,
			waitedMs: 1500,
		});
		expect(done.durationMs).toBe(2850);
	});

	test("a stale answer after speech resumed is ignored", () => {
		const h = harness();
		h.push(join(tone(1000, 0.3), silence(200)));
		const stale = h.asks[0].id;
		h.push(tone(400, 0.3));
		expect(h.segmenter.answerTurn(stale, 0.9)).toEqual([]);
		h.push(silence(200));
		expect(h.asks).toHaveLength(2);
	});

	test("an unanswered check ends the turn by the silence rule and its late answer is ignored", () => {
		const h = harness();
		expect(h.push(join(tone(1000, 0.3), silence(400)))).toEqual([]);
		const [done] = h.push(silence(100));
		expect(done.turn).toMatchObject({ source: "silence", waitedMs: 500 });
		expect(h.segmenter.answerTurn(h.asks[0].id, 0.1)).toEqual([]);
		expect(h.segmenter.answerTurn(h.asks[0].id, 0.9)).toEqual([]);
	});

	test("without a turn model the utterance ends at 450 ms of silence", () => {
		const declined = createUtteranceSegmenter({ onTurnCheck: () => false });
		const withoutModel = (segmenter: typeof declined) => {
			const out: Utterance[] = [];
			for (const part of [tone(1000, 0.3), silence(500)]) {
				for (let at = 0; at < part.length; at += CHUNK) {
					const chunk = part.subarray(at, at + CHUNK);
					out.push(...segmenter.push(chunk, speechOf(chunk)));
				}
			}
			return out;
		};
		const missing = harness({ turnModel: false });
		missing.push(tone(1000, 0.3));
		expect(missing.push(silence(400))).toEqual([]);
		expect(missing.push(silence(100))[0].turn).toMatchObject({
			source: "silence",
			waitedMs: 500,
		});
		expect(withoutModel(declined)[0].turn?.source).toBe("silence");
	});

	test("a null answer falls back to the 450 ms rule", () => {
		const h = harness();
		h.push(join(tone(1000, 0.3), silence(200)));
		expect(h.segmenter.answerTurn(h.asks[0].id, null)).toEqual([]);
		expect(h.push(silence(200))).toEqual([]);
		const [done] = h.push(silence(100));
		expect(done.turn).toMatchObject({ source: "silence", waitedMs: 500 });
	});

	test("continuous speech is cut at 15 s and flush reports its source", () => {
		const h = harness();
		const [cut] = h.push(tone(15_000, 0.3));
		expect(cut.turn?.source).toBe("max");
		h.push(tone(500, 0.3));
		expect(h.segmenter.flush()?.turn?.source).toBe("flush");
	});

	test("chunks without a probability follow the energy rules, mid-stream", () => {
		const segmenter = createUtteranceSegmenter({ onTurnCheck: () => true });
		const out = feed(segmenter, join(tone(700, 0.3), silence(500)));
		expect(out).toHaveLength(1);
		expect(out[0].turn).toMatchObject({ source: "silence", waitedMs: 500 });
	});

	test("reset drops a pending turn check", () => {
		const h = harness();
		h.push(join(tone(1000, 0.3), silence(200)));
		h.segmenter.reset();
		expect(h.segmenter.answerTurn(h.asks[0].id, 0.9)).toEqual([]);
	});
});

describe("createBargeInDetector with a voice-activity model", () => {
	const detect = (signal: Float32Array, probability?: number) => {
		const detector = createBargeInDetector();
		const hits: Float32Array[][] = [];
		for (let at = 0; at < signal.length; at += CHUNK) {
			const hit = detector.push(signal.subarray(at, at + CHUNK), probability);
			if (hit) hits.push(hit);
		}
		return hits;
	};

	test("250 ms of loud speech starts a barge-in", () => {
		expect(detect(tone(200, 0.4), 0.9)).toEqual([]);
		expect(detect(tone(300, 0.4), 0.9)).toHaveLength(1);
	});

	test("loud audio the model calls non-speech does not", () => {
		expect(detect(tone(3000, 0.4), 0.3)).toEqual([]);
	});

	test("speech the model hears but that is below the barge threshold does not", () => {
		expect(detect(tone(3000, 0.04), 0.95)).toEqual([]);
	});

	test("the window is 500 ms", () => {
		const gap = join(
			tone(100, 0.4),
			silence(600),
			tone(100, 0.4),
			silence(600),
		);
		expect(detect(gap, 0.9)).toEqual([]);
	});

	test("without a probability the loud-only rule applies", () => {
		expect(detect(tone(200, 0.4))).toEqual([]);
		expect(detect(tone(300, 0.4))).toHaveLength(1);
	});
});

describe("model endpointing constants", () => {
	test("match the contract", () => {
		expect(MIN_SPEECH_MS).toBe(250);
		expect(TURN_CHECK_SILENCE_MS).toBe(200);
		expect(INCOMPLETE_TURN_SILENCE_MS).toBe(1500);
		expect(VAD_START_PROBABILITY).toBe(0.5);
		expect(VAD_CONTINUE_PROBABILITY).toBe(0.35);
		expect(BARGE_IN_CONFIRM_MS).toBe(500);
		expect(BARGE_IN_VAD_PROBABILITY).toBe(0.6);
	});
});
