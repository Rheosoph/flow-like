import { describe, expect, test } from "bun:test";
import { getPackageOverviewHref } from "./package-navigation";

describe("getPackageOverviewHref", () => {
	test("removes package detail state", () => {
		expect(getPackageOverviewHref(new URLSearchParams("id=package-1"))).toBe(
			"/store/packages",
		);
	});

	test("preserves the originating package tab", () => {
		const params = new URLSearchParams(
			"tab=installed&id=package-1&purchase=success",
		);

		expect(getPackageOverviewHref(params)).toBe(
			"/store/packages?tab=installed",
		);
	});
});
