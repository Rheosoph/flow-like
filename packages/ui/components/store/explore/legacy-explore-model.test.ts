import { describe, expect, test } from "bun:test";
import {
	LEGACY_PACKAGES_HREF,
	legacyAppsQuery,
	legacyFallbackTarget,
} from "./legacy-explore-model";

const query = (search: string) => legacyAppsQuery(new URLSearchParams(search));

describe("legacyAppsQuery", () => {
	test("translates Browse params into the old apps page's vocabulary", () => {
		expect(
			query(
				"type=apps&q=invoice&categories=package:EDUCATION,app:Finance,app:Health&sort=rating",
			).toString(),
		).toBe("type=apps&q=invoice&category=Finance&sort=rated");
	});

	test("keeps sorts both pages share and drops the ones the old page lacks", () => {
		expect(query("sort=newest").toString()).toBe("sort=newest");
		expect(query("sort=updated").toString()).toBe("sort=updated");
		for (const sort of ["best", "installs", "name", "popular", "bogus"]) {
			expect(query(`sort=${sort}`).toString()).toBe("");
		}
	});

	test("is idempotent on the old page's own URLs", () => {
		const legacy = "q=crm&category=Business&sort=rated";
		expect(query(legacy).toString()).toBe(legacy);
		expect(query(query(legacy).toString()).toString()).toBe(legacy);
		const typed = "type=packages&q=crm";
		expect(query(typed).toString()).toBe(typed);
	});

	test("keeps only the types the fallback acts on", () => {
		expect(query("type=packages").get("type")).toBe("packages");
		expect(query("type=apps").get("type")).toBe("apps");
		for (const type of ["all", "collections", "bogus"]) {
			expect(query(`type=${type}`).has("type")).toBe(false);
		}
	});

	test("prefers a valid legacy category and ignores unknown ones", () => {
		expect(
			query("category=Finance&categories=app:Health").get("category"),
		).toBe("Finance");
		expect(query("category=Nope&categories=app:Health").get("category")).toBe(
			"Health",
		);
		expect(query("categories=app:Nope,package:EDUCATION").has("category")).toBe(
			false,
		);
	});

	test("drops filters without a legacy equivalent", () => {
		expect(
			query(
				"collection=abc&price=paid&verified=true&permissions=network&type=collections",
			).toString(),
		).toBe("");
		expect(query("q=%20%20").toString()).toBe("");
	});
});

describe("legacyFallbackTarget", () => {
	const target = (developerMode: boolean, search: string) =>
		legacyFallbackTarget({
			developerMode,
			pathname: "/store/explore/search",
			searchParams: new URLSearchParams(search),
		});

	test("packages go to the old package list in developer mode", () => {
		expect(target(true, "type=packages")).toBe(LEGACY_PACKAGES_HREF);
		expect(target(true, "type=packages&q=pdf")).toBe(LEGACY_PACKAGES_HREF);
	});

	test("without developer mode packages read as apps and keep their type", () => {
		expect(target(false, "type=packages&categories=app:Finance")).toBe(
			"/store/explore/search?type=packages&category=Finance",
		);
		expect(target(false, "type=packages&category=Finance")).toBeNull();
	});

	test("a Browse URL is rewritten in place", () => {
		expect(target(true, "type=apps&categories=app:Finance&sort=rating")).toBe(
			"/store/explore/search?type=apps&category=Finance&sort=rated",
		);
		expect(target(true, "type=all")).toBe("/store/explore/search");
	});

	test("a URL already in legacy form needs no rewrite, whatever its order", () => {
		expect(target(true, "")).toBeNull();
		expect(target(true, "type=apps")).toBeNull();
		expect(target(false, "sort=rated&q=crm&category=Business")).toBeNull();
	});
});
