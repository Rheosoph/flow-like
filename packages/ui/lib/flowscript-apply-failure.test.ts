import { describe, expect, test } from "bun:test";
import { flowScriptApplyOutcome } from "./flowscript-apply-failure";

describe("flowScriptApplyOutcome", () => {
	test("an apply that did what the source asked is not captured", () => {
		expect(flowScriptApplyOutcome(12, 0)).toBeUndefined();
		expect(flowScriptApplyOutcome(0, 0)).toBeUndefined();
	});

	test("diagnostics without commands are blocked", () => {
		expect(flowScriptApplyOutcome(0, 1)).toBe("blocked");
	});

	test("diagnostics alongside commands are partial", () => {
		expect(flowScriptApplyOutcome(4, 2)).toBe("partial");
	});
});
