import { fileURLToPath } from "node:url";
import { describe, expect, test } from "vitest";

import {
	loadDocScreenshotPlan,
	validateDocScreenshotPlan,
	validateDocScreenshotTauriFixture,
} from "../plan";
import {
	DOC_SCREENSHOT_PLAN_SCHEMA,
	DOC_SCREENSHOT_TAURI_FIXTURE_SCHEMA,
} from "../types";

function planWithSteps(
	steps: unknown[],
	overrides: Record<string, unknown> = {},
) {
	return {
		schema: DOC_SCREENSHOT_PLAN_SCHEMA,
		app: "desktop",
		outputDir: "tmp/doc-screenshots",
		scenarios: [
			{
				name: "example",
				path: "/onboarding",
				steps,
			},
		],
		...overrides,
	};
}

describe("document screenshot plan validation", () => {
	test("validates the checked-in onboarding example", async () => {
		const examplePath = fileURLToPath(
			new URL("../examples/onboarding.plan.json", import.meta.url),
		);
		const plan = await loadDocScreenshotPlan(examplePath);

		expect(plan.schema).toBe(DOC_SCREENSHOT_PLAN_SCHEMA);
		expect(plan.defaults.viewport).toEqual({
			width: 1624,
			height: 1060,
			deviceScaleFactor: 2,
		});
		expect(plan.scenarios).toHaveLength(1);
		expect(
			plan.scenarios[0]?.steps
				.filter((step) => step.type === "capture")
				.map((step) => step.name),
		).toEqual(["overview", "selected", "complete"]);
	});

	test("rejects capture output traversal", () => {
		expect(() =>
			validateDocScreenshotPlan(
				planWithSteps([
					{
						type: "capture",
						name: "escape",
						output: "../escape.png",
					},
				]),
			),
		).toThrow("must stay inside outputDir");
	});

	test("rejects duplicate capture names", () => {
		expect(() =>
			validateDocScreenshotPlan(
				planWithSteps([
					{ type: "capture", name: "same" },
					{ type: "capture", name: "same" },
				]),
			),
		).toThrow("Duplicate capture name: same");
	});

	test("rejects an element capture without a selector", () => {
		expect(() =>
			validateDocScreenshotPlan(
				planWithSteps([{ type: "capture", name: "card", mode: "element" }]),
			),
		).toThrow("selector is required for element capture");
	});

	test("rejects viewports above the output pixel limit", () => {
		expect(() =>
			validateDocScreenshotPlan(
				planWithSteps([{ type: "capture", name: "oversized" }], {
					defaults: {
						viewport: {
							width: 7680,
							height: 7680,
							deviceScaleFactor: 2,
						},
					},
				}),
			),
		).toThrow("exceeds the 100 megapixel capture limit");
	});

	test("rejects unsupported actions", () => {
		expect(() =>
			validateDocScreenshotPlan(
				planWithSteps([
					{ type: "drag", selector: "#source", target: "#destination" },
					{ type: "capture", name: "after-drag" },
				]),
			),
		).toThrow("type is not supported: drag");
	});
});

describe("Tauri screenshot fixture validation", () => {
	test("defaults strict fixture handling to true", () => {
		const fixture = validateDocScreenshotTauriFixture({
			schema: DOC_SCREENSHOT_TAURI_FIXTURE_SCHEMA,
			responses: {
				get_profile: { id: "docs", enabled: true },
			},
		});

		expect(fixture.strict).toBe(true);
		expect(fixture.responses).toEqual({
			get_profile: { id: "docs", enabled: true },
		});
	});

	test.each([
		["undefined", undefined],
		["NaN", Number.NaN],
		["bigint", BigInt(1)],
	])("rejects a non-JSON %s response", (_label, response) => {
		expect(() =>
			validateDocScreenshotTauriFixture({
				schema: DOC_SCREENSHOT_TAURI_FIXTURE_SCHEMA,
				responses: { invalid: response },
			}),
		).toThrow("fixture.responses.invalid is not JSON-serializable");
	});
});
