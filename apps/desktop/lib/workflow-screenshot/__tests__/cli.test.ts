import { resolve } from "node:path";
import { describe, expect, test } from "vitest";
import {
	inferWorkflowScreenshotFormat,
	parseWorkflowScreenshotArgs,
} from "../cli";

describe("workflow screenshot CLI", () => {
	test("parses the book rendering controls", () => {
		const options = parseWorkflowScreenshotArgs([
			"apps/book/examples/incident-triage/triage.flow",
			"--output=apps/book/src/assets/workflows/triage.png",
			"--layout=expanded",
			"--focus-node",
			"normalize",
			"--viewport=1440x900",
			"--dpr=2.5",
			"--theme=light",
		]);

		expect(options).toMatchObject({
			input: resolve(
				process.cwd(),
				"apps/book/examples/incident-triage/triage.flow",
			),
			output: resolve(
				process.cwd(),
				"apps/book/src/assets/workflows/triage.png",
			),
			layout: "expanded",
			focusNode: "normalize",
			viewport: { width: 1440, height: 900 },
			dpr: 2.5,
			theme: "light",
		});
	});

	test("defaults to a balanced lossless WebP capture", () => {
		const options = parseWorkflowScreenshotArgs(["examples/sample.flow"]);
		expect(options.layout).toBe("balanced");
		expect(options.output).toBe(
			resolve(process.cwd(), "tmp/workflow-screenshots/sample.webp"),
		);
		expect(options.viewport).toEqual({ width: 1624, height: 1060 });
		expect(options.dpr).toBe(2);
	});

	test("validates layout, output format, and JPEG quality", () => {
		expect(() =>
			parseWorkflowScreenshotArgs(["sample.flow", "--layout=diagonal"]),
		).toThrow("--layout must be compact, balanced, or expanded.");
		expect(() =>
			parseWorkflowScreenshotArgs([
				"sample.flow",
				"--output=sample.webp",
				"--quality=80",
			]),
		).toThrow("--quality requires JPEG output.");
		expect(inferWorkflowScreenshotFormat("sample.jpeg")).toBe("jpeg");
	});
});
