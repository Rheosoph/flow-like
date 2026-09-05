import { describe, expect, test } from "bun:test";
import { getInsertionPlacement } from "./dropPlacement";

const container = { left: 100, top: 100, width: 400, height: 300 };
const children = [
	{ index: 0, left: 120, top: 120, width: 360, height: 50 },
	{ index: 2, left: 120, top: 220, width: 360, height: 80 },
];

describe("canvas insertion placement", () => {
	test("uses actual child indexes when hidden children are absent", () => {
		expect(
			getInsertionPlacement(
				container,
				children,
				{ x: 200, y: 230 },
				{ orientation: "vertical" },
			).index,
		).toBe(2);
		expect(
			getInsertionPlacement(
				container,
				children,
				{ x: 200, y: 290 },
				{ orientation: "vertical" },
			).index,
		).toBe(3);
	});

	test("chooses the nearest insertion boundary across gaps", () => {
		expect(
			getInsertionPlacement(
				container,
				children,
				{ x: 200, y: 180 },
				{ orientation: "vertical" },
			).index,
		).toBe(1);
		expect(
			getInsertionPlacement(
				container,
				children,
				{ x: 200, y: 210 },
				{ orientation: "vertical" },
			).index,
		).toBe(2);
	});

	test("returns the visible screen position at a fractional zoom", () => {
		const scaled = (rect: typeof container) => ({
			left: rect.left * 0.625 + 20,
			top: rect.top * 0.625 + 40,
			width: rect.width * 0.625,
			height: rect.height * 0.625,
		});
		const placement = getInsertionPlacement(
			scaled(container),
			children.map((child) => ({ ...scaled(child), index: child.index })),
			{ x: 145, y: 221.25 },
			{ orientation: "vertical" },
		);
		expect(placement.indicator).toEqual({
			left: 82.5,
			top: 226.5,
			width: 250,
			height: 2,
		});
	});

	test("uses both axes for wrapped rows and grids", () => {
		const grid = [
			{ index: 0, left: 100, top: 100, width: 80, height: 80 },
			{ index: 1, left: 200, top: 100, width: 80, height: 80 },
			{ index: 2, left: 100, top: 200, width: 80, height: 80 },
			{ index: 3, left: 200, top: 200, width: 80, height: 80 },
		];
		const placement = getInsertionPlacement(
			container,
			grid,
			{ x: 150, y: 250 },
			{ orientation: "horizontal", wrapped: true },
		);
		expect(placement.index).toBe(3);
		expect(placement.indicator).toEqual({
			left: 179,
			top: 200,
			width: 2,
			height: 80,
		});
	});

	test("respects reversed flex direction", () => {
		const row = [{ index: 0, left: 300, top: 100, width: 100, height: 80 }];
		expect(
			getInsertionPlacement(
				container,
				row,
				{ x: 310, y: 110 },
				{ orientation: "horizontal", reverse: true },
			).index,
		).toBe(1);
		expect(
			getInsertionPlacement(
				container,
				row,
				{ x: 390, y: 110 },
				{ orientation: "horizontal", reverse: true },
			).index,
		).toBe(0);
	});
});
