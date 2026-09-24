import { describe, expect, test } from "bun:test";
import { exploreSearchFixture } from "./explore-fixture";
import {
	type ExploreBrowseParams,
	exploreHref,
	exploreSearchHref,
	exploreSearchText,
	exploreTypeFromParam,
	legacyAppsExploreTarget,
	parseBrowseParams,
	serializeBrowseParams,
} from "./explore-href";
import {
	APP_CATEGORIES,
	EXPLORE_SEARCH_LIMITS,
	WASM_PACKAGE_CATEGORIES,
} from "./explore-types";

function roundTrip(params: ExploreBrowseParams): ExploreBrowseParams {
	return parseBrowseParams(new URLSearchParams(serializeBrowseParams(params)));
}

function legacy(query: string): string {
	return legacyAppsExploreTarget(new URLSearchParams(query));
}

describe("exploreHref", () => {
	test("links the landing and preselects a type", () => {
		expect(exploreHref()).toBe("/store/explore");
		expect(exploreHref({ type: "all" })).toBe("/store/explore");
		expect(exploreHref({ type: "apps" })).toBe("/store/explore?type=apps");
		expect(exploreHref({ type: "packages" })).toBe(
			"/store/explore?type=packages",
		);
	});

	test("exploreTypeFromParam falls back to all", () => {
		expect(exploreTypeFromParam("apps")).toBe("apps");
		expect(exploreTypeFromParam("packages")).toBe("packages");
		expect(exploreTypeFromParam("all")).toBe("all");
		expect(exploreTypeFromParam("collections")).toBe("all");
		expect(exploreTypeFromParam(null)).toBe("all");
		expect(exploreTypeFromParam(undefined)).toBe("all");
	});
});

describe("browse params", () => {
	test("round-trip with comma lists", () => {
		const params: ExploreBrowseParams = {
			type: "packages",
			q: "invoice",
			categories: ["app:Finance", "package:EDUCATION"],
			price: "paid",
			verified: true,
			permissions: ["network", "models"],
			sort: "rating",
			collection: "collection-invoices",
		};
		const serialized = serializeBrowseParams(params);
		expect(serialized).toBe(
			"type=packages&q=invoice&categories=app:Finance,package:EDUCATION&price=paid&verified=true&permissions=network,models&sort=rating&collection=collection-invoices",
		);
		expect(roundTrip(params)).toEqual(params);
	});

	test("an empty selection serializes to nothing", () => {
		expect(serializeBrowseParams({})).toBe("");
		expect(exploreSearchHref()).toBe("/store/explore/search");
		expect(roundTrip({})).toEqual({});
		expect(
			roundTrip({ q: "  ", categories: [], permissions: [], verified: false }),
		).toEqual({});
	});

	test("a query with reserved characters survives", () => {
		const params: ExploreBrowseParams = { q: "a&b=c, d:e + f/100%" };
		expect(roundTrip(params)).toEqual(params);
	});

	test("every category facet value shape parses back", () => {
		for (const category of APP_CATEGORIES) {
			const params: ExploreBrowseParams = { categories: [`app:${category}`] };
			expect(roundTrip(params)).toEqual(params);
		}
		for (const category of WASM_PACKAGE_CATEGORIES) {
			const params: ExploreBrowseParams = {
				categories: [`package:${category}`],
			};
			expect(roundTrip(params)).toEqual(params);
		}
		expect(
			roundTrip({ categories: ["app:Finance", "package:DOCUMENT_PROCESSING"] }),
		).toEqual({ categories: ["app:Finance", "package:DOCUMENT_PROCESSING"] });
	});

	test("facet values emitted by the search endpoint select themselves", () => {
		const values = exploreSearchFixture().facets.categories.map(
			(facet) => facet.value,
		);
		expect(values.length).toBeGreaterThan(0);
		expect(
			parseBrowseParams(new URLSearchParams(`categories=${values.join(",")}`))
				.categories,
		).toEqual(values);
	});

	test("unknown, mis-cased and duplicate values are dropped", () => {
		const parsed = parseBrowseParams(
			new URLSearchParams(
				"type=everything&categories=app:Finance,Finance,package:Education,app:Nope,app:Finance&price=cheap&verified=yes&permissions=network,foo,network&sort=relevance",
			),
		);
		expect(parsed).toEqual({
			categories: ["app:Finance"],
			permissions: ["network"],
		});
	});

	test("encoded separators parse like readable ones", () => {
		expect(
			parseBrowseParams(
				new URLSearchParams(
					"categories=app%3AFinance%2Cpackage%3AEDUCATION&permissions=none%2Cstorage",
				),
			),
		).toEqual({
			categories: ["app:Finance", "package:EDUCATION"],
			permissions: ["none", "storage"],
		});
	});

	test("collections is a valid type", () => {
		expect(roundTrip({ type: "collections" })).toEqual({ type: "collections" });
	});
});

describe("search limits", () => {
	const allFacets = [
		...APP_CATEGORIES.map((category) => `app:${category}`),
		...WASM_PACKAGE_CATEGORIES.map((category) => `package:${category}`),
	];

	test("a pasted query is cut to the server's 100 characters", () => {
		const parsed = parseBrowseParams(
			new URLSearchParams({ q: ` ${"x".repeat(150)} ` }),
		);
		expect(parsed.q).toBe("x".repeat(EXPLORE_SEARCH_LIMITS.q));
	});

	test("the cut never splits a character", () => {
		const q = exploreSearchText(`${"a".repeat(99)}😀😀`);
		expect(Array.from(q ?? "")).toHaveLength(EXPLORE_SEARCH_LIMITS.q);
		expect(q?.endsWith("😀")).toBe(true);
		expect(() =>
			serializeBrowseParams({ q: `${"a".repeat(99)}😀` }),
		).not.toThrow();
	});

	test("a shared link with more than 16 facets keeps the first 16", () => {
		expect(allFacets.length).toBeGreaterThan(EXPLORE_SEARCH_LIMITS.categories);
		const parsed = parseBrowseParams(
			new URLSearchParams(`categories=${allFacets.join(",")}`),
		);
		expect(parsed.categories).toEqual(
			allFacets.slice(0, EXPLORE_SEARCH_LIMITS.categories),
		);
	});

	test("serializing caps the lists after dropping duplicates", () => {
		const params = new URLSearchParams(
			serializeBrowseParams({
				categories: [allFacets[0], allFacets[1], allFacets[0], ...allFacets],
				q: "y".repeat(120),
			}),
		);
		expect(params.get("categories")?.split(",")).toEqual(
			allFacets.slice(0, EXPLORE_SEARCH_LIMITS.categories),
		);
		expect(params.get("q")).toHaveLength(EXPLORE_SEARCH_LIMITS.q);
	});

	test("a long legacy query lands on Browse within the limit", () => {
		const target = legacy(`q=${"z".repeat(140)}`);
		const query = new URLSearchParams(target.slice(target.indexOf("?") + 1));
		expect(query.get("q")).toHaveLength(EXPLORE_SEARCH_LIMITS.q);
	});
});

describe("legacyAppsExploreTarget", () => {
	test("bare → the curated landing with the Apps filter", () => {
		expect(legacy("")).toBe("/store/explore?type=apps");
	});

	test("sort=popular only → the curated landing", () => {
		expect(legacy("sort=popular")).toBe("/store/explore?type=apps");
	});

	test("query, category and rated sort → Browse", () => {
		expect(legacy("q=x&category=Finance&sort=rated")).toBe(
			"/store/explore/search?type=apps&q=x&categories=app:Finance&sort=rating",
		);
	});

	test("newest and updated sorts → Browse", () => {
		expect(legacy("sort=newest")).toBe(
			"/store/explore/search?type=apps&sort=newest",
		);
		expect(legacy("sort=updated")).toBe(
			"/store/explore/search?type=apps&sort=updated",
		);
	});

	test("popular alongside a filter maps to best", () => {
		expect(legacy("q=invoice&sort=popular")).toBe(
			"/store/explore/search?type=apps&q=invoice&sort=best",
		);
	});

	test("a category alone → Browse", () => {
		expect(legacy("category=Productivity")).toBe(
			"/store/explore/search?type=apps&categories=app:Productivity",
		);
	});

	test("values the old page ignored stay ignored", () => {
		expect(legacy("q=%20%20&category=Nope&sort=bogus")).toBe(
			"/store/explore?type=apps",
		);
		expect(legacy("sort=toString")).toBe("/store/explore?type=apps");
	});

	test("the Browse target parses back to the same selection", () => {
		const target = legacy("q=x&category=Finance&sort=rated");
		const query = target.slice(target.indexOf("?") + 1);
		expect(parseBrowseParams(new URLSearchParams(query))).toEqual({
			type: "apps",
			q: "x",
			categories: ["app:Finance"],
			sort: "rating",
		});
	});
});
