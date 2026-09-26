import { describe, expect, test } from "bun:test";
import {
	type GuideBox,
	pickReferences,
	snapToGuides,
	unionBox,
} from "./flow-helper-lines";

const box = (x: number, y: number, width = 100, height = 50): GuideBox => ({
	x,
	y,
	width,
	height,
});

const SCREEN = box(-10_000, -10_000, 20_000, 20_000);

describe("snapToGuides", () => {
	test("snaps a left edge within the threshold", () => {
		const snap = snapToGuides(box(104, 300), [box(100, 0)], { threshold: 6 });
		expect(snap.dx).toBe(-4);
		expect(snap.lines).toContainEqual({ axis: "x", at: 100, from: 0, to: 350 });
	});

	test("ignores alignment beyond the threshold", () => {
		const snap = snapToGuides(box(110, 300), [box(100, 0)], { threshold: 6 });
		expect(snap).toEqual({ dx: 0, dy: 0, lines: [] });
	});

	test("matches like-for-like anchors only", () => {
		const snap = snapToGuides(box(201, 300), [box(100, 0)], { threshold: 6 });
		expect(snap.dx).toBe(0);
	});

	test("aligns centres of differently sized boxes", () => {
		const snap = snapToGuides(box(123, 300, 60), [box(100, 0, 100)], {
			threshold: 6,
		});
		expect(snap.dx).toBe(-3);
		expect(snap.lines.map((line) => line.at)).toEqual([150]);
	});

	test("prefers the nearer reference when offsets tie", () => {
		const far = box(100, -2000);
		const near = box(96, 200);
		const snap = snapToGuides(box(98, 300), [far, near], { threshold: 6 });
		expect(snap.dx).toBe(-2);
	});

	test("draws a guide only to the nearest aligned reference", () => {
		const snap = snapToGuides(box(100, 300), [box(100, -2000), box(100, 0)], {
			threshold: 6,
		});
		const left = snap.lines.filter(
			(line) => line.axis === "x" && line.at === 100,
		);
		expect(left).toEqual([{ axis: "x", at: 100, from: 0, to: 350 }]);
	});

	test("a straight wire outranks a closer box edge", () => {
		const snap = snapToGuides(box(400, 101), [box(0, 100)], {
			threshold: 6,
			pins: [{ x: 400, y: 121, targetX: 100, targetY: 125 }],
		});
		expect(snap.dy).toBe(4);
		expect(snap.lines).toContainEqual({
			axis: "y",
			at: 125,
			from: 100,
			to: 400,
			pin: true,
		});
	});

	test("leaves a locked axis alone", () => {
		const snap = snapToGuides(box(104, 3), [box(100, 0)], {
			threshold: 6,
			axes: { x: false, y: true },
		});
		expect(snap.dx).toBe(0);
		expect(snap.dy).toBe(-3);
		expect(snap.lines.every((line) => line.axis === "y")).toBe(true);
	});
});

describe("pickReferences", () => {
	test("keeps only the nearest on-screen boxes", () => {
		const moving = box(0, 0);
		const candidates = [box(0, 100), box(0, 400), box(0, 900), box(0, 5000)];
		const picked = pickReferences(candidates, moving, {
			visible: box(-500, -500, 2000, 2000),
			limit: 2,
		});
		expect(picked).toEqual([box(0, 100), box(0, 400)]);
	});

	test("always keeps wired boxes beyond the nearest", () => {
		const moving = box(0, 0);
		const wired = box(0, 900);
		const picked = pickReferences([box(0, 100), box(0, 400), wired], moving, {
			visible: SCREEN,
			limit: 1,
			always: new Set([wired]),
		});
		expect(picked).toEqual([wired, box(0, 100)]);
	});

	test("skips boxes nested with the dragged one", () => {
		const moving = box(50, 50);
		const frame = box(0, 0, 400, 400);
		const inner = box(60, 60, 10, 10);
		const picked = pickReferences([frame, inner, box(600, 0)], moving, {
			visible: SCREEN,
			limit: 5,
		});
		expect(picked).toEqual([box(600, 0)]);
	});
});

test("unionBox spans every box", () => {
	expect(unionBox([box(0, 0), box(200, 100, 50, 50)])).toEqual(
		box(0, 0, 250, 150),
	);
	expect(unionBox([])).toBeUndefined();
});
