import { describe, expect, it } from "vitest";
import { expectedCopilotPinType } from "./copilot-command-pins";
import { IPinType } from "./schema/flow/pin";

describe("expectedCopilotPinType", () => {
	it("keeps ordinary node directions", () => {
		expect(expectedCopilotPinType(IPinType.Input, false)).toBe(IPinType.Input);
		expect(expectedCopilotPinType(IPinType.Output, false)).toBe(
			IPinType.Output,
		);
	});

	it("inverts function-layer boundary directions", () => {
		expect(expectedCopilotPinType(IPinType.Output, true)).toBe(IPinType.Input);
		expect(expectedCopilotPinType(IPinType.Input, true)).toBe(IPinType.Output);
	});
});
