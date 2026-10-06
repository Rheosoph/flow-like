import { describe, expect, test } from "bun:test";
import { NO_DRAG, dropKind } from "./file-model";

const over = (count: number | null) => ({ over: true, count });

describe("what dropping files would do (spec M3)", () => {
	test("nothing is said while nothing is dragged over", () => {
		expect(dropKind(NO_DRAG, true, false)).toBeNull();
		expect(dropKind(NO_DRAG, true, true)).toBeNull();
	});

	test("several files over a one-file field start one run each", () => {
		expect(dropKind(over(10), true, false)).toBe("many");
		expect(dropKind(over(10), true, true)).toBe("many");
		expect(dropKind(over(2), true, false)).toBe("many");
	});

	test("an unknown count says so on the drop row, a single file says nothing", () => {
		expect(dropKind(over(null), true, false)).toBe("unknown");
		expect(dropKind(over(1), true, false)).toBeNull();
	});

	test("over an attached file one file replaces it", () => {
		expect(dropKind(over(1), true, true)).toBe("replace");
		expect(dropKind(over(null), true, true)).toBe("replace");
	});

	test("a host without next files promises nothing", () => {
		expect(dropKind(over(10), false, false)).toBeNull();
		expect(dropKind(over(null), false, false)).toBeNull();
		expect(dropKind(over(1), false, true)).toBe("replace");
	});
});
