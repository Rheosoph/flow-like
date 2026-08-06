import { describe, expect, it } from "bun:test";
import {
	applyComposerLean,
	createComposerActivity,
	createTypingResponse,
	decayPerk,
	stepTypingResponse,
} from "./composer-activity";

describe("createComposerActivity", () => {
	it("bumps the revision on every change to the draft", () => {
		const channel = createComposerActivity();
		channel.report("h");
		channel.report("he");
		expect(channel.read().revision).toBe(2);
		expect(channel.read().length).toBe(2);
	});

	it("ignores a repeat of the same draft, so a re-render is not a keystroke", () => {
		const channel = createComposerActivity();
		channel.report("hello");
		channel.report("hello");
		channel.report("hello");
		expect(channel.read().revision).toBe(1);
	});

	it("counts an edit that leaves the length alone", () => {
		const channel = createComposerActivity();
		channel.report("cat");
		channel.report("bat");
		expect(channel.read().revision).toBe(2);
		expect(channel.read().length).toBe(3);
	});

	it("reports the clear that follows a send", () => {
		const channel = createComposerActivity();
		channel.report("ship it");
		channel.report("");
		expect(channel.read().length).toBe(0);
		expect(channel.read().revision).toBe(2);
	});

	it("keeps surfaces independent, so one chat cannot stir another's mark", () => {
		const a = createComposerActivity();
		const b = createComposerActivity();
		a.report("typing over here");
		expect(b.read().revision).toBe(0);
		expect(b.read().length).toBe(0);
	});
});

/** Runs the accumulator for `seconds` at `fps`, striking a key every `keysPerSecond`. */
function run(start: number, seconds: number, keysPerSecond: number, fps = 60) {
	const dt = 1 / fps;
	const frames = Math.round(seconds * fps);
	const framesPerKey =
		keysPerSecond > 0 ? fps / keysPerSecond : Number.POSITIVE_INFINITY;
	let typing = start;
	let sinceKey = framesPerKey;
	for (let i = 0; i < frames; i++) {
		sinceKey += 1;
		const keyed = sinceKey >= framesPerKey;
		if (keyed) sinceKey = 0;
		typing = stepTypingResponse(typing, keyed, dt);
	}
	return typing;
}

describe("stepTypingResponse", () => {
	it("pins near 1 while someone is actually writing", () => {
		expect(run(0, 1, 8)).toBeGreaterThan(0.9);
	});

	it("only lifts partway for hunt-and-peck, so slow typing stays a slow mark", () => {
		const slow = run(0, 3, 2);
		expect(slow).toBeGreaterThan(0.25);
		expect(slow).toBeLessThan(0.8);
	});

	it("barely registers a single key", () => {
		expect(stepTypingResponse(0, true, 1 / 60)).toBeLessThan(0.35);
	});

	it("drains inside a couple of seconds once typing stops", () => {
		expect(run(1, 1, 0)).toBeLessThan(0.25);
		expect(run(1, 2, 0)).toBeLessThan(0.06);
		expect(run(1, 4, 0)).toBeLessThan(0.01);
	});

	it("never leaves the 0…1 range the pose blend expects", () => {
		expect(run(0, 5, 30)).toBeLessThanOrEqual(1);
		expect(run(0, 5, 0)).toBeGreaterThanOrEqual(0);
	});

	it("decays on wall-clock time, so a 120Hz display is not a faster mark", () => {
		expect(run(1, 1.5, 0, 120)).toBeCloseTo(run(1, 1.5, 0, 60), 3);
		expect(run(1, 0.4, 0, 240)).toBeCloseTo(run(1, 0.4, 0, 30), 3);
	});

	it("holds the same ceiling at either refresh rate", () => {
		// The exact value samples a sawtooth — where in the gap between keys the frame lands —
		// so the invariant is the level it settles at, not one frame of it.
		expect(run(0, 1.5, 8, 120)).toBeGreaterThan(0.8);
		expect(run(0, 1.5, 8, 60)).toBeGreaterThan(0.8);
	});
});

const FRAME = 1 / 60;

describe("applyComposerLean", () => {
	const RADIUS = 0.48;

	it("passes the aim straight through when nothing is being typed", () => {
		const lean = applyComposerLean(0.2, -0.1, 0, 3, RADIUS);
		expect(lean.x).toBe(0.2);
		expect(lean.y).toBe(-0.1);
	});

	it("pulls the bulge onto the lower rim at full typing, whatever it was aimed at", () => {
		// The composer is always below the mark, so y must end up negative and near the rim.
		for (const [x, y] of [
			[0, 0.4],
			[-0.5, 0.5],
			[0.3, 0],
		]) {
			const lean = applyComposerLean(x, y, 1, 3, RADIUS);
			expect(lean.y).toBeCloseTo(-RADIUS * 0.9, 10);
			expect(Math.abs(lean.x)).toBeLessThanOrEqual(RADIUS * 0.22 + 1e-9);
		}
	});

	it("stays inside the film rather than flying off it", () => {
		for (let clock = 0; clock < 20; clock += 0.37) {
			for (const typing of [0.25, 0.5, 0.75, 1]) {
				const lean = applyComposerLean(0.6, 0.6, typing, clock, RADIUS);
				expect(Math.hypot(lean.x, lean.y)).toBeLessThan(1);
			}
		}
	});

	it("keeps the lean alive with a sway instead of pinning it dead centre", () => {
		const seen = new Set<number>();
		for (let clock = 0; clock < 6; clock += 0.25) {
			seen.add(Math.round(applyComposerLean(0, 0, 1, clock, RADIUS).x * 1e6));
		}
		expect(seen.size).toBeGreaterThan(5);
	});
});

describe("createTypingResponse", () => {
	it("stays at rest when the interface never enabled it", () => {
		const channel = createComposerActivity();
		const response = createTypingResponse(channel);
		for (let i = 0; i < 60; i++) {
			channel.report(`draft ${i}`);
			const step = response.advance(false, FRAME);
			expect(step.typing).toBe(0);
			expect(step.perked).toBe(false);
			expect(step.fullness).toBe(0);
		}
	});

	it("perks once at the start of a burst, not on every key", () => {
		const channel = createComposerActivity();
		const response = createTypingResponse(channel);
		let perks = 0;
		for (let i = 0; i < 60; i++) {
			channel.report("x".repeat(i + 1));
			if (response.advance(true, FRAME).perked) perks += 1;
		}
		expect(perks).toBe(1);
	});

	it("perks again once the composer has been quiet long enough", () => {
		const channel = createComposerActivity();
		const response = createTypingResponse(channel);
		channel.report("a");
		expect(response.advance(true, FRAME).perked).toBe(true);
		// Quiet for well over the gap.
		for (let i = 0; i < 90; i++) response.advance(true, FRAME);
		channel.report("ab");
		expect(response.advance(true, FRAME).perked).toBe(true);
	});

	it("does not perk on the clear that follows a send", () => {
		const channel = createComposerActivity();
		const response = createTypingResponse(channel);
		channel.report("send me");
		response.advance(true, FRAME);
		for (let i = 0; i < 90; i++) response.advance(true, FRAME);
		channel.report("");
		expect(response.advance(true, FRAME).perked).toBe(false);
	});

	it("fills toward 1 as the draft grows and eases back when it is cleared", () => {
		const channel = createComposerActivity();
		const response = createTypingResponse(channel);
		channel.report("x".repeat(400));
		for (let i = 0; i < 240; i++) response.advance(true, FRAME);
		expect(response.advance(true, FRAME).fullness).toBeGreaterThan(0.9);

		channel.report("");
		for (let i = 0; i < 240; i++) response.advance(true, FRAME);
		expect(response.advance(true, FRAME).fullness).toBeLessThan(0.05);
	});

	it("keeps the perk impulse in the 0…1 range a swell can be scaled by", () => {
		let perk = 0;
		for (let i = 0; i < 600; i++) {
			perk = decayPerk(perk, i % 7 === 0, FRAME);
			expect(perk).toBeLessThanOrEqual(1);
			expect(perk).toBeGreaterThanOrEqual(0);
		}
	});

	it("decays the perk on wall-clock time, so a fast display is not a faster blink", () => {
		const perkAfter = (fps: number, seconds: number) => {
			let perk = decayPerk(0, true, 1 / fps);
			for (let i = 0; i < Math.round(seconds * fps); i++) {
				perk = decayPerk(perk, false, 1 / fps);
			}
			return perk;
		};
		expect(perkAfter(120, 0.6)).toBeCloseTo(perkAfter(60, 0.6), 3);
		// Fully settled well before the next burst can begin (PERK_GAP is 0.9s).
		expect(perkAfter(60, 0.9)).toBeLessThan(0.05);
	});

	it("eases back rather than snapping when the setting is turned off mid-draft", () => {
		const channel = createComposerActivity();
		const response = createTypingResponse(channel);
		for (let i = 0; i < 60; i++) {
			channel.report("x".repeat(i + 1));
			response.advance(true, FRAME);
		}
		const off = response.advance(false, FRAME);
		expect(off.typing).toBeGreaterThan(0.5);
		expect(off.fullness).toBeGreaterThan(0);

		for (let i = 0; i < 300; i++) response.advance(false, FRAME);
		const settled = response.advance(false, FRAME);
		expect(settled.typing).toBeLessThan(0.01);
		expect(settled.fullness).toBeLessThan(0.01);
	});
});
