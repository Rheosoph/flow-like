import { describe, expect, it } from "bun:test";
import { createHomeWidget } from "./catalog";
import { homeInsertionIndex, insertHomeWidget } from "./home-drag";
import {
	homeWidgetAutoHeight,
	homeWidgetHeight,
	normalizeHomeLayout,
} from "./home-layout";

const widgets = ["a", "b", "c"].map((id) => ({
	...createHomeWidget("information"),
	id,
}));
const canvas = { left: 0, top: 0, right: 1000, bottom: 900 };
const rects = [
	{ id: "a", left: 0, top: 0, right: 1000, bottom: 100 },
	{ id: "b", left: 0, top: 116, right: 490, bottom: 316 },
	{ id: "c", left: 506, top: 116, right: 1000, bottom: 516 },
];

describe("home drop placement", () => {
	it("places above and below a wide widget using its vertical midpoint", () => {
		expect(
			homeInsertionIndex(widgets, "new", { x: 800, y: 20 }, canvas, rects),
		).toBe(0);
		expect(
			homeInsertionIndex(widgets, "new", { x: 200, y: 80 }, canvas, rects),
		).toBe(1);
	});
	it("places alongside unequal cards without scaling their bounds", () => {
		expect(
			homeInsertionIndex(widgets, "new", { x: 40, y: 200 }, canvas, rects),
		).toBe(1);
		expect(
			homeInsertionIndex(widgets, "new", { x: 470, y: 200 }, canvas, rects),
		).toBe(2);
		expect(
			homeInsertionIndex(widgets, "new", { x: 970, y: 200 }, canvas, rects),
		).toBe(3);
	});
	it("holds the preview in place while the pointer is inside it", () => {
		expect(
			homeInsertionIndex(widgets, "b", { x: 470, y: 200 }, canvas, rects),
		).toBe(1);
		expect(
			homeInsertionIndex(widgets, "b", { x: 30, y: 200 }, canvas, rects),
		).toBe(1);
	});
	it("supports an empty canvas, appending, and cancelling outside", () => {
		expect(homeInsertionIndex([], "new", { x: 30, y: 200 }, canvas, [])).toBe(
			0,
		);
		expect(
			homeInsertionIndex(widgets, "b", { x: 500, y: 800 }, canvas, rects),
		).toBe(2);
		expect(
			homeInsertionIndex(widgets, "b", { x: 1100, y: 100 }, canvas, rects),
		).toBeNull();
	});
	it("moves once without duplicating identity or mutating the saved order", () => {
		expect(
			insertHomeWidget(widgets, widgets[0], 2).map((widget) => widget.id),
		).toEqual(["b", "c", "a"]);
		expect(widgets.map((widget) => widget.id)).toEqual(["a", "b", "c"]);
	});
});

describe("home content sizing", () => {
	it("uses responsive height by default and preserves explicit sizing across serialization", () => {
		expect(homeWidgetAutoHeight(widgets[0])).toBe(true);
		const fixed = {
			...widgets[0],
			size: { columns: 6, rows: 3, heightMode: "fixed", height: 288 },
		};
		const restored = normalizeHomeLayout(
			JSON.parse(JSON.stringify({ version: 1, widgets: [fixed] })),
		)?.widgets[0];
		expect(restored).toBeDefined();
		if (!restored) throw new Error("Expected a restored layout");
		expect(homeWidgetAutoHeight(restored)).toBe(false);
		expect(homeWidgetHeight(restored)).toBe(288);
	});
	it("bounds imported dimensions", () => {
		const restored = normalizeHomeLayout({
			version: 1,
			widgets: [
				{
					...widgets[0],
					size: { columns: 99, rows: 99, height: 99999, heightMode: "fixed" },
				},
			],
		})?.widgets[0];
		expect(restored?.size).toEqual({
			columns: 12,
			rows: 12,
			height: 1240,
			heightMode: "fixed",
		});
	});
});
