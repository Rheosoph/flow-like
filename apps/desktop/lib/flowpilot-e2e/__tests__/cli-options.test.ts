import { describe, expect, test } from "vitest";

import { parseArgs } from "../../../scripts/flowpilot-e2e";

describe("FlowPilot E2E CLI options", () => {
	test("parses an ordered explicit subset and tight-loop controls", () => {
		expect(
			parseArgs([
				"--case",
				"forum",
				"--case=simple-agent",
				"--repeat",
				"3",
				"--min-chars=900",
				"--fail-fast",
				"--json",
			]),
		).toMatchObject({
			caseIds: ["forum", "simple-agent"],
			repeat: 3,
			minChars: 900,
			failFast: true,
			json: true,
		});
	});

	test("rejects ambiguous and expensive accidental selections", () => {
		expect(() => parseArgs(["--case", "forum", "--suite", "smoke"])).toThrow(
			"either --case or --suite",
		);
		expect(() => parseArgs(["--repeat", "21"])).toThrow("cannot exceed 20");
		expect(() => parseArgs(["--case", "unknown"])).toThrow("Unknown");
	});
});
