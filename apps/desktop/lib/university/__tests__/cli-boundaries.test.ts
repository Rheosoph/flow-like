import { describe, expect, test } from "vitest";

import { parseUniversityArgs } from "../cli";

describe("University CLI option boundaries", () => {
	test.each(["1", "300000"])("accepts timeout boundary %s", (timeout) => {
		expect(
			parseUniversityArgs(["--list", `--timeout-ms=${timeout}`]).timeoutMs,
		).toBe(Number(timeout));
	});

	test.each(["0", "1.5", "300001"])("rejects unsafe timeout %s", (timeout) => {
		expect(() =>
			parseUniversityArgs(["--list", `--timeout-ms=${timeout}`]),
		).toThrow("--timeout-ms");
	});

	test("limits language to course reads", () => {
		expect(() =>
			parseUniversityArgs(["--plan=course.plan.json", "--language=de"]),
		).toThrow("--language can only be used with --inspect or --list");
	});

	test.each(["--replace", "--kind=IMAGE", "--mime-type=image/png"])(
		"requires asset mode for %s",
		(assetOption) => {
			expect(() => parseUniversityArgs(["--list", assetOption])).toThrow(
				"require --asset",
			);
		},
	);

	test("rejects unexpected positional values", () => {
		expect(() => parseUniversityArgs(["--list", "course-id"])).toThrow(
			"Unknown argument: course-id",
		);
	});

	test("rejects an invalid direct-upload MIME type", () => {
		expect(() =>
			parseUniversityArgs([
				"--asset=course-id",
				"--name=Shot",
				"--file=shot.png",
				"--mime-type=not-a-mime",
			]),
		).toThrow("valid MIME type");
	});
});
