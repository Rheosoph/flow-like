import { describe, expect, test } from "bun:test";
import { placeElementToolbar } from "./element-geometry";

const viewport = { left: 300, top: 100, width: 500, height: 400 };
const toolbar = { width: 260, height: 36 };

describe("element action placement", () => {
	test("stays above an element when there is enough room", () => {
		const placed = placeElementToolbar(
			{ left: 350, top: 200, width: 100, height: 50 },
			toolbar,
			viewport,
		);
		expect(placed.left).toBe(350);
		expect(placed.top + placed.height).toBe(194);
		expect(placed.visible).toBe(true);
	});

	test("flips below the element at the top edge and clamps away from the right edge", () => {
		const placed = placeElementToolbar(
			{ left: 760, top: 100, width: 30, height: 20 },
			toolbar,
			viewport,
		);
		expect(placed.top).toBe(126);
		expect(placed.left + placed.width).toBe(794);
	});

	test("keeps actions within the visible scroll viewport for a tall selected container", () => {
		const placed = placeElementToolbar(
			{ left: 200, top: 50, width: 700, height: 700 },
			toolbar,
			viewport,
		);
		expect(placed.left).toBe(306);
		expect(placed.top).toBe(106);
		expect(placed.top + placed.height).toBeLessThanOrEqual(494);
	});

	test("constrains wrapping actions to a narrow canvas without clipping them", () => {
		const placed = placeElementToolbar(
			{ left: 305, top: 200, width: 40, height: 30 },
			{ width: 260, height: 110 },
			{ ...viewport, width: 130 },
		);
		expect(placed.maxWidth).toBe(118);
		expect(placed.left).toBe(306);
		expect(placed.left + placed.width).toBe(424);
		expect(placed.top + placed.height).toBeLessThanOrEqual(494);
	});

	test("hides actions for a selected element scrolled out of view", () => {
		expect(
			placeElementToolbar(
				{ left: 350, top: 0, width: 100, height: 20 },
				toolbar,
				viewport,
			).visible,
		).toBe(false);
	});
});
