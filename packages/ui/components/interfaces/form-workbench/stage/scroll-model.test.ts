import { describe, expect, test } from "bun:test";
import {
	CARET_GAP,
	activeSectionOf,
	askingTurn,
	followsTail,
	sectionScrollTop,
	tailNudge,
} from "./scroll-model";

const tops = (...values: [string, number][]) =>
	values.map(([key, top]) => ({ key: key as "steps", top }));

describe("activeSectionOf", () => {
	const probe = (over: object = {}) => ({
		tops: tops(["steps", 0], ["answer", 300], ["inputs", 900]),
		viewHeight: 600,
		scrollTop: 0,
		scrollHeight: 2000,
		...over,
	});

	test("the first section while the body is at the top", () => {
		expect(activeSectionOf(probe())).toBe("steps");
	});

	test("at the top the first section stays current even when a later one is already past the line", () => {
		expect(
			activeSectionOf(
				probe({ tops: tops(["steps", 24], ["answer", 118], ["inputs", 250]) }),
			),
		).toBe("steps");
	});

	test("a section counts once its top passes 40 % of the view", () => {
		expect(
			activeSectionOf(
				probe({
					scrollTop: 10,
					tops: tops(["steps", -300], ["answer", 239], ["inputs", 600]),
				}),
			),
		).toBe("answer");
		expect(
			activeSectionOf(
				probe({
					scrollTop: 10,
					tops: tops(["steps", -300], ["answer", 241], ["inputs", 600]),
				}),
			),
		).toBe("steps");
	});

	test("never closer than 48 px for a short view", () => {
		expect(
			activeSectionOf(
				probe({
					scrollTop: 10,
					viewHeight: 80,
					tops: tops(["steps", -10], ["answer", 47], ["inputs", 49]),
				}),
			),
		).toBe("answer");
	});

	test("at the very end of a scrolled body the last section is current", () => {
		expect(
			activeSectionOf(
				probe({
					scrollTop: 1400,
					tops: tops(["steps", -1300], ["answer", -700], ["inputs", 500]),
				}),
			),
		).toBe("inputs");
	});

	test("an unscrolled short body keeps the first section", () => {
		expect(activeSectionOf(probe({ scrollHeight: 600 }))).toBe("steps");
	});

	test("no sections: none", () => {
		expect(activeSectionOf(probe({ tops: [] }))).toBeNull();
	});
});

describe("following a live answer", () => {
	test("followed while the end is in view or below, not once scrolled up past it", () => {
		expect(followsTail(400, 600)).toBe(true);
		expect(followsTail(700, 600)).toBe(false);
		expect(followsTail(null, 600)).toBe(true);
	});

	test("the nudge keeps the end 88 px above the bottom edge, where the canvas keeps its caret, and never scrolls back", () => {
		expect(CARET_GAP).toBe(88);
		expect(tailNudge(650, 600)).toBe(138);
		expect(tailNudge(513, 600)).toBe(1);
		expect(tailNudge(512, 600)).toBe(0);
		expect(tailNudge(500, 600)).toBe(0);
	});

	test("another gap can be asked for", () => {
		expect(tailNudge(650, 600, 24)).toBe(74);
	});

	test("a run that starts asking shows its question; once answered it is followed again", () => {
		expect(askingTurn(false, true)).toBe("reveal");
		expect(askingTurn(true, false)).toBe("resume");
		expect(askingTurn(true, true)).toBeNull();
		expect(askingTurn(false, false)).toBeNull();
	});
});

describe("sectionScrollTop", () => {
	test("puts the section's top 20 px below the body's top", () => {
		expect(sectionScrollTop(100, 420)).toBe(500);
		expect(sectionScrollTop(0, 10)).toBe(0);
	});
});
