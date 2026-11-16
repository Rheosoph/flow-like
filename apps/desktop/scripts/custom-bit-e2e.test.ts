import { describe, expect, test } from "bun:test";
import {
	type LiveReport,
	requiredCases,
	validateReport,
} from "./custom-bit-e2e";

function report(): LiveReport {
	return {
		schema: "flowpilot.custom-bit-e2e/v1",
		runId: "run",
		passed: true,
		cases: requiredCases.map((id) => {
			const provider = id.split(":")[0];
			return {
				id,
				provider,
				bitId: `${provider}-bit`,
				modelId: `${provider}-model`,
				passed: true,
				evidence: { resolvedBitId: `${provider}-bit` },
			};
		}),
	};
}

describe("live custom-Bit report validation", () => {
	test("accepts a complete matrix with consistent concrete models", () => {
		expect(validateReport(report(), "run").passed).toBe(true);
	});
	test("recomputes failure instead of trusting the webview summary", () => {
		const value = report();
		value.cases[2].passed = false;
		expect(validateReport(value, "run").passed).toBe(false);
	});
	test("rejects missing or duplicated paths in final results", () => {
		const value = report();
		value.cases.pop();
		expect(() => validateReport(value, "run")).toThrow("omitted");
		value.cases.push(value.cases[0]);
		expect(() => validateReport(value, "run")).toThrow("duplicate");
	});
	test("allows partial checkpoints without declaring completion", () => {
		const value = report();
		value.cases = value.cases.slice(0, 2);
		expect(validateReport(value, "run", false).passed).toBe(false);
	});
	test("rejects model substitution and evidence-free success", () => {
		const changed = report();
		changed.cases[1].bitId = "other-bit";
		expect(() => validateReport(changed, "run")).toThrow(
			"Selected model changed",
		);
		const absent = report();
		absent.cases[0].evidence = {};
		expect(() => validateReport(absent, "run")).toThrow(
			"no concrete model evidence",
		);
		const implicit = report();
		implicit.cases[0].modelId = "default";
		expect(() => validateReport(implicit, "run")).toThrow(
			"no concrete model evidence",
		);
	});
	test("rejects callbacks for a different run", () => {
		expect(() => validateReport(report(), "other-run")).toThrow(
			"does not match",
		);
	});
});
