import { resolve } from "node:path";
import { describe, expect, test } from "vitest";

import { parseUniversityArgs } from "../cli";

describe("University CLI arguments", () => {
	test("parses an offline plan dry run", () => {
		expect(
			parseUniversityArgs(["--plan=course.plan.json", "--dry-run", "--json"]),
		).toMatchObject({
			mode: "apply",
			plan: resolve(process.cwd(), "course.plan.json"),
			dryRun: true,
			json: true,
		});
	});

	test("parses direct screenshot asset upload options", () => {
		expect(
			parseUniversityArgs([
				"--asset",
				"course-basics",
				"--name=EditorOverview",
				"--file",
				"tmp/doc-screenshots/editor.webp",
				"--kind=image",
				"--mime-type=image/webp",
				"--replace",
			]),
		).toMatchObject({
			mode: "asset",
			assetCourseId: "course-basics",
			assetName: "EditorOverview",
			assetFile: resolve(process.cwd(), "tmp/doc-screenshots/editor.webp"),
			assetKind: "IMAGE",
			assetMimeType: "image/webp",
			replaceAsset: true,
		});
	});

	test("keeps operation modes mutually exclusive", () => {
		expect(() =>
			parseUniversityArgs(["--list", "--inspect", "course-basics"]),
		).toThrow("cannot be combined");
	});

	test("requires explicit asset metadata without accepting unsafe names", () => {
		expect(() =>
			parseUniversityArgs([
				"--asset=course-basics",
				"--name=bad asset",
				"--file=shot.webp",
			]),
		).toThrow("--name must start");
		expect(() =>
			parseUniversityArgs(["--asset=course-basics", "--name=ValidName"]),
		).toThrow("requires --asset <course-id>, --name, and --file");
	});

	test("does not allow an authentication token argument", () => {
		expect(() =>
			parseUniversityArgs(["--list", "--token=pat_secret.value"]),
		).toThrow("Unknown argument: --token");
	});
});
