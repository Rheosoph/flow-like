import { describe, expect, test } from "bun:test";
import {
	chipMove,
	chipText,
	entersChips,
	itemsFrom,
	neighbour,
} from "./chips-model";

describe("where the cursor goes between chips", () => {
	test("← and → move, → past the last chip goes back to the entry, ← on the first does nothing", () => {
		expect(chipMove("ArrowLeft", 2, 3)).toEqual({ kind: "focus", to: 1 });
		expect(chipMove("ArrowLeft", 0, 3)).toBeNull();
		expect(chipMove("ArrowRight", 0, 3)).toEqual({ kind: "focus", to: 1 });
		expect(chipMove("ArrowRight", 2, 3)).toEqual({
			kind: "focus",
			to: "entry",
		});
		expect(chipMove("x", 1, 3)).toBeNull();
	});

	test("⌫ removes and the cursor goes to the previous chip, Delete to the next one", () => {
		expect(chipMove("Backspace", 2, 3)).toEqual({ kind: "remove", after: 1 });
		expect(chipMove("Backspace", 0, 3)).toEqual({ kind: "remove", after: 0 });
		expect(chipMove("Delete", 0, 3)).toEqual({ kind: "remove", after: 0 });
		expect(chipMove("Delete", 2, 3)).toEqual({ kind: "remove", after: 1 });
	});

	test("the last chip removed leaves the cursor in the entry", () => {
		expect(chipMove("Backspace", 0, 1)).toEqual({
			kind: "remove",
			after: "entry",
		});
		expect(chipMove("Delete", 0, 1)).toEqual({
			kind: "remove",
			after: "entry",
		});
		expect(neighbour(3, 0)).toBe("entry");
		expect(neighbour(5, 3)).toBe(2);
		expect(neighbour(-1, 3)).toBe(0);
	});

	test("← and ⌫ in an empty entry go into the chips, nothing else does", () => {
		expect(entersChips("ArrowLeft", "", 2)).toBe(true);
		expect(entersChips("Backspace", "", 2)).toBe(true);
		expect(entersChips("Backspace", "x", 2)).toBe(false);
		expect(entersChips("Backspace", "", 0)).toBe(false);
		expect(entersChips("ArrowRight", "", 2)).toBe(false);
	});
});

describe("what an entry adds", () => {
	const never = () => null;

	test("text is split at commas, trimmed, and empty parts are dropped", () => {
		expect(itemsFrom(" 4420 , 4430,, ", { itemKind: "text" }, never)).toEqual([
			"4420",
			"4430",
		]);
		expect(itemsFrom("", { itemKind: "text" }, never)).toEqual([]);
		expect(itemsFrom("12,5", { itemKind: "number" }, never)).toEqual([
			"12",
			"5",
		]);
	});

	test("dates are read in the viewer's order; one that cannot be read adds nothing", () => {
		const read = (typed: string) => (typed === "18/10" ? "2026-10-18" : null);
		expect(itemsFrom("18/10", { itemKind: "date" }, read)).toEqual([
			"2026-10-18",
		]);
		expect(itemsFrom("18/10, nope", { itemKind: "date" }, read)).toBeNull();
	});

	test("a chip shows a day in the viewer's format", () => {
		expect(chipText("text", "4400", "en-GB")).toBe("4400");
		expect(chipText("number", "12.5", "en-GB")).toBe("12.5");
		expect(chipText("date", "2026-10-18", "en-GB")).toContain("2026");
	});
});
