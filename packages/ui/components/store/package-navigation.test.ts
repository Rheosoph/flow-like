import { describe, expect, test } from "bun:test";
import {
	getPackageOverviewHref,
	packageStoreHref,
	safeStoreReturnPath,
} from "./package-navigation";

const EXPLORE_SEARCH = "/store/explore/search?q=invoice";

function overviewFrom(from: string, rest = "id=package-1") {
	const params = new URLSearchParams(rest);
	params.set("from", from);
	return getPackageOverviewHref(params);
}

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

	test("returns to the library tab without the package id", () => {
		expect(
			getPackageOverviewHref(new URLSearchParams("tab=library&id=x")),
		).toBe("/store/packages?tab=library");
	});

	test("round-trips an Explore search through packageStoreHref", () => {
		const href = packageStoreHref({ id: "package-1", from: EXPLORE_SEARCH });
		const query = new URL(href, "https://app.example").searchParams;

		expect(query.get("id")).toBe("package-1");
		expect(getPackageOverviewHref(query)).toBe(EXPLORE_SEARCH);
	});

	test("returns a safe from even when a tab is present", () => {
		expect(overviewFrom(EXPLORE_SEARCH, "tab=mine&id=package-1")).toBe(
			EXPLORE_SEARCH,
		);
	});

	test.each([
		"//evil.com",
		"//evil.com/store",
		"https://evil.com/store",
		"/settings",
		"/storefront",
		"/store/../settings",
		"/\\evil.com/store",
		"/\t/evil.com/store",
		"javascript:alert(1)",
	])("ignores unsafe from %p", (from) => {
		expect(overviewFrom(from, "tab=library&id=x")).toBe(
			"/store/packages?tab=library",
		);
	});
});

describe("safeStoreReturnPath", () => {
	test("accepts the store root and paths under it", () => {
		expect(safeStoreReturnPath("/store")).toBe("/store");
		expect(
			safeStoreReturnPath("/store/package-workspace?id=a&tab=listing"),
		).toBe("/store/package-workspace?id=a&tab=listing");
	});

	test("drops the fragment", () => {
		expect(safeStoreReturnPath("/store/explore#rail")).toBe("/store/explore");
	});

	test("rejects empty and relative values", () => {
		expect(safeStoreReturnPath(null)).toBeNull();
		expect(safeStoreReturnPath(undefined)).toBeNull();
		expect(safeStoreReturnPath("")).toBeNull();
		expect(safeStoreReturnPath("store/explore")).toBeNull();
	});
});

describe("packageStoreHref", () => {
	test("builds the detail href with an optional tab", () => {
		expect(packageStoreHref({ id: "pkg" })).toBe("/store/packages?id=pkg");
		expect(packageStoreHref({ id: "pkg", tab: "mine" })).toBe(
			"/store/packages?id=pkg&tab=mine",
		);
	});

	test("encodes the id and the return path", () => {
		const href = packageStoreHref({
			id: "a b&c",
			from: "/store/package-workspace?id=a&tab=listing",
		});
		const query = new URL(href, "https://app.example").searchParams;

		expect(query.get("id")).toBe("a b&c");
		expect(query.get("from")).toBe("/store/package-workspace?id=a&tab=listing");
	});

	test("never carries an unsafe from", () => {
		expect(packageStoreHref({ id: "pkg", from: "//evil.com" })).toBe(
			"/store/packages?id=pkg",
		);
	});
});
