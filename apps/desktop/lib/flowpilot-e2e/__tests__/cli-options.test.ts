import { describe, expect, test } from "vitest";

import { constantTimeEqual, parseArgs } from "../../../scripts/flowpilot-e2e";

describe("FlowPilot E2E CLI options", () => {
	test("compares callback capabilities without throwing on length mismatch", () => {
		expect(constantTimeEqual("/callback-secret", "/callback-secret")).toBe(
			true,
		);
		expect(constantTimeEqual("/callback-secrex", "/callback-secret")).toBe(
			false,
		);
		expect(() => constantTimeEqual("/short", "/callback-secret")).not.toThrow();
		expect(constantTimeEqual("/short", "/callback-secret")).toBe(false);
	});

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
			modelKey: "terra",
			repeat: 3,
			minChars: 900,
			failFast: true,
			json: true,
		});
	});

	test("pins the benchmark model by alias or model id", () => {
		expect(
			parseArgs(["--case", "ai-adventure", "--model", "sol"]),
		).toMatchObject({ caseIds: ["ai-adventure"], modelKey: "sol" });
		expect(parseArgs(["--model=gpt-5.6-sol"])).toMatchObject({
			modelKey: "sol",
		});
	});

	test("runs cases in parallel only when fail-fast is not requested", () => {
		expect(parseArgs(["--suite", "smoke", "--concurrency", "3"])).toMatchObject(
			{
				suite: "smoke",
				concurrency: 3,
			},
		);
		expect(parseArgs([])).toMatchObject({ concurrency: 1 });
		expect(() => parseArgs(["--concurrency", "5"])).toThrow("cannot exceed 4");
		expect(() => parseArgs(["--concurrency", "2", "--fail-fast"])).toThrow(
			"needs sequential cases",
		);
	});

	test("rejects ambiguous and expensive accidental selections", () => {
		expect(() => parseArgs(["--case", "forum", "--suite", "smoke"])).toThrow(
			"either --case or --suite",
		);
		expect(() => parseArgs(["--repeat", "21"])).toThrow("cannot exceed 20");
		expect(() => parseArgs(["--case", "unknown"])).toThrow("Unknown");
		expect(() => parseArgs(["--model", "gpt-4"])).toThrow(
			"Unknown FlowPilot E2E model",
		);
	});
});
