import { describe, expect, test } from "bun:test";
import {
	ZOOM_MAX,
	ZOOM_MIN,
	edgeNudge,
	fitView,
	panBy,
	topView,
	wheelPixels,
	wheelZoomFactor,
	zoomAt,
} from "./org-chart-viewport";

const toScreen = (view: { x: number; y: number; k: number }, x: number) =>
	view.x + x * view.k;

describe("org chart viewport", () => {
	test("zooming keeps the point under the cursor in place", () => {
		const view = { x: 40, y: -20, k: 1 };
		const next = zoomAt(view, 1.5, 300, 200);
		expect(next.k).toBe(1.5);
		const chartX = (300 - view.x) / view.k;
		expect(toScreen(next, chartX)).toBeCloseTo(300);
		const chartY = (200 - view.y) / view.k;
		expect(next.y + chartY * next.k).toBeCloseTo(200);
	});

	test("zoom stops at its limits without drifting", () => {
		const atMax = { x: 10, y: 10, k: ZOOM_MAX };
		expect(zoomAt(atMax, ZOOM_MAX * 2, 100, 100)).toEqual(atMax);
		expect(zoomAt({ x: 0, y: 0, k: 1 }, 0.01, 0, 0).k).toBe(ZOOM_MIN);
	});

	test("panning moves the view by the drag distance", () => {
		expect(panBy({ x: 5, y: 5, k: 2 }, 10, -15)).toEqual({
			x: 15,
			y: -10,
			k: 2,
		});
	});

	test("fit centres the chart and never enlarges past 100%", () => {
		expect(
			fitView({ width: 1000, height: 600 }, { width: 2000, height: 600 }),
		).toEqual({ k: 0.5, x: 0, y: 150 });
		expect(
			fitView({ width: 1000, height: 600 }, { width: 400, height: 200 }),
		).toEqual({ k: 1, x: 300, y: 200 });
	});

	test("the first view fits the width and starts at the top", () => {
		expect(
			topView({ width: 800, height: 300 }, { width: 400, height: 900 }),
		).toEqual({ k: 1, x: 200, y: 0 });
		expect(
			topView({ width: 800, height: 300 }, { width: 1600, height: 900 }),
		).toEqual({ k: 0.5, x: 0, y: 0 });
	});

	test("wheel deltas become pixels and shift scrolls sideways", () => {
		const base = { deltaX: 0, deltaY: 3, shiftKey: false };
		expect(wheelPixels({ ...base, deltaMode: 1 }, 500)).toEqual({
			dx: 0,
			dy: 48,
		});
		expect(wheelPixels({ ...base, deltaMode: 2 }, 500).dy).toBe(1500);
		expect(wheelPixels({ ...base, deltaMode: 0, shiftKey: true }, 500)).toEqual(
			{ dx: 3, dy: 0 },
		);
	});

	test("one mouse-wheel notch zooms by a bounded step", () => {
		expect(wheelZoomFactor(-4)).toBeGreaterThan(1);
		expect(wheelZoomFactor(4)).toBeLessThan(1);
		expect(wheelZoomFactor(1000)).toBe(wheelZoomFactor(40));
		expect(wheelZoomFactor(100)).toBeGreaterThan(0.7);
	});

	test("dragging near an edge nudges harder the closer it gets", () => {
		expect(edgeNudge(60, 48, 14)).toBe(0);
		expect(edgeNudge(0, 48, 14)).toBe(14);
		expect(edgeNudge(24, 48, 14)).toBe(7);
		expect(edgeNudge(-10, 48, 14)).toBe(14);
	});
});
