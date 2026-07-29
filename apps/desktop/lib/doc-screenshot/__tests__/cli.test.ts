import { basename, dirname, resolve } from "node:path";
import { describe, expect, test } from "vitest";

import { directPlanFromOptions, parseDocScreenshotArgs } from "../cli";

describe("document screenshot CLI", () => {
	test("parses repeated query values and direct capture rendering options", () => {
		const options = parseDocScreenshotArgs([
			"--app=web",
			"--path",
			"/onboarding",
			"--query",
			"section=welcome",
			"--query=section=profiles",
			"--viewport=1440x900",
			"--dpr",
			"2.5",
			"--theme=dark",
			"--full-page",
			"--output",
			"tmp/docs/onboarding.png",
		]);

		expect(options).toMatchObject({
			app: "web",
			path: "/onboarding",
			query: [
				["section", "welcome"],
				["section", "profiles"],
			],
			viewport: { width: 1440, height: 900 },
			dpr: 2.5,
			theme: "dark",
			fullPage: true,
			output: resolve(process.cwd(), "tmp/docs/onboarding.png"),
		});
	});

	test("builds a validated direct plan with query arrays, DPR, mode, and inferred output format", () => {
		const output = resolve(process.cwd(), "tmp/docs/onboarding.png");
		const plan = directPlanFromOptions(
			parseDocScreenshotArgs([
				"--path=/onboarding",
				"--query=tag=first",
				"--query=tag=second",
				"--query=empty=",
				"--viewport=1280x720",
				"--dpr=3",
				"--full-page",
				`--output=${output}`,
			]),
		);

		expect(plan.outputDir).toBe(dirname(output));
		expect(plan.defaults.viewport).toEqual({
			width: 1280,
			height: 720,
			deviceScaleFactor: 3,
		});
		expect(plan.defaults.format).toBe("png");
		expect(plan.scenarios).toHaveLength(1);
		expect(plan.scenarios[0]).toMatchObject({
			path: "/onboarding",
			query: {
				tag: ["first", "second"],
				empty: [""],
			},
		});
		expect(plan.scenarios[0]?.steps).toEqual([
			{
				type: "capture",
				name: "onboarding",
				output: basename(output),
				mode: "fullPage",
				selector: undefined,
				format: "png",
				quality: undefined,
			},
		]);
	});

	test("keeps full-page and element capture mutually exclusive", () => {
		expect(() =>
			parseDocScreenshotArgs([
				"--path=/onboarding",
				"--full-page",
				"--selector=main",
			]),
		).toThrow("Use either --full-page or --selector, not both.");
	});

	test("rejects an explicit format that disagrees with the output extension", () => {
		const options = parseDocScreenshotArgs([
			"--path=/onboarding",
			"--format=webp",
			"--output=onboarding.png",
		]);

		expect(() => directPlanFromOptions(options)).toThrow(
			"--format does not match the --output extension.",
		);
	});
});
