import { beforeAll, describe, expect, test } from "bun:test";
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { unzipSync, zipSync } from "fflate";
import { pack } from "../src/pack";
import { validateBundle, validateProject } from "../src/validate-cmd";
import { type ProjectFixture, makeProjectFixture, tmpDir } from "./helpers";

describe("validateProject", () => {
	test("accepts the synthetic project", () => {
		const fixture = makeProjectFixture();
		const result = validateProject(fixture.projectDir);
		expect(result.errors).toEqual([]);
		expect(result.ok).toBeTrue();
		expect(result.widgets).toEqual([{ id: "hello-widget", group: "react" }]);
	}, 60000);

	test("reports a missing flow-like.toml", () => {
		const result = validateProject(tmpDir("flwb-empty"));
		expect(result.ok).toBeFalse();
		expect(result.errors.some((e) => e.includes("flow-like.toml"))).toBeTrue();
	});
});

describe("validateBundle", () => {
	let fixture: ProjectFixture;
	let flwbPath: string;
	let bytes: Uint8Array;

	beforeAll(async () => {
		fixture = makeProjectFixture();
		flwbPath = join(fixture.projectDir, "widgets.flwb");
		({ bytes } = await pack(fixture.projectDir, {
			out: flwbPath,
			quiet: true,
		}));
	});

	test("accepts a freshly packed bundle", () => {
		const result = validateBundle(flwbPath);
		expect(result.errors).toEqual([]);
		expect(result.ok).toBeTrue();
		expect(result.manifest?.widgets[0]?.id).toBe("hello-widget");
	}, 60000);

	test("catches a tampered entry", () => {
		const entries = unzipSync(bytes);
		entries["widgets/hello-widget/index.html"] = new TextEncoder().encode(
			"<html>tampered</html>",
		);
		const tamperedPath = join(tmpDir("flwb-tamper"), "tampered.flwb");
		writeFileSync(tamperedPath, zipSync(entries));

		const result = validateBundle(tamperedPath);
		expect(result.ok).toBeFalse();
		expect(
			result.errors.some((e) =>
				e.includes(
					"Hash mismatch for widget entry widgets/hello-widget/index.html",
				),
			),
		).toBeTrue();
	}, 60000);

	test("reports a missing bundle file", () => {
		const result = validateBundle("/nonexistent/widgets.flwb");
		expect(result.ok).toBeFalse();
		expect(result.errors[0]).toContain("not found");
	});
});
