import { describe, expect, test } from "bun:test";
import { PackageStatus } from "../../lib/schema/wasm";
import {
	LEGACY_TAB_ALIASES,
	filterRegistryMine,
	hubExploreAction,
	hubHref,
	hubTabFromParam,
	isLegacyHubTab,
	registryMineCounts,
	registryMineFilterKeys,
	registryMineState,
	sortRegistryMine,
} from "./packages-hub-model";

describe("hubTabFromParam", () => {
	test("keeps the hub tabs", () => {
		expect(hubTabFromParam("mine")).toBe("mine");
		expect(hubTabFromParam("library")).toBe("library");
		expect(hubTabFromParam("explore")).toBe("explore");
	});

	test("no tab is Explore", () => {
		expect(hubTabFromParam(null)).toBe("explore");
		expect(hubTabFromParam(undefined)).toBe("explore");
		expect(hubTabFromParam("")).toBe("explore");
	});

	test("maps the legacy aliases", () => {
		expect(hubTabFromParam("projects")).toBe("mine");
		expect(hubTabFromParam("installed")).toBe("library");
		expect(LEGACY_TAB_ALIASES).toEqual({
			projects: "mine",
			installed: "library",
		});
	});

	test("unknown and prototype keys fall back to Explore", () => {
		expect(hubTabFromParam("settings")).toBe("explore");
		expect(hubTabFromParam("constructor")).toBe("explore");
		expect(hubTabFromParam("__proto__")).toBe("explore");
		expect(hubTabFromParam("toString")).toBe("explore");
	});
});

describe("isLegacyHubTab", () => {
	test("only the aliases are legacy", () => {
		expect(isLegacyHubTab("projects")).toBe(true);
		expect(isLegacyHubTab("installed")).toBe(true);
		expect(isLegacyHubTab("mine")).toBe(false);
		expect(isLegacyHubTab("constructor")).toBe(false);
		expect(isLegacyHubTab(null)).toBe(false);
	});
});

describe("hubExploreAction", () => {
	test("waits while support is unknown", () => {
		expect(hubExploreAction(undefined)).toBe("wait");
	});

	test("redirects when the hub has Explore (also on 401, offline and 5xx)", () => {
		expect(hubExploreAction(true)).toBe("redirect");
	});

	test("falls back to the legacy list on a hub without Explore", () => {
		expect(hubExploreAction(false)).toBe("legacy");
	});
});

describe("hubHref", () => {
	test("sets the tab and keeps other params", () => {
		expect(hubHref(new URLSearchParams("q=pdf"), "library")).toBe(
			"/store/packages?q=pdf&tab=library",
		);
		expect(hubHref(new URLSearchParams("tab=projects"), "mine")).toBe(
			"/store/packages?tab=mine",
		);
	});

	test("Explore drops the tab", () => {
		expect(hubHref(new URLSearchParams("tab=mine"), "explore")).toBe(
			"/store/packages",
		);
		expect(hubHref(new URLSearchParams("tab=library&q=a"), "explore")).toBe(
			"/store/packages?q=a",
		);
	});

	test("does not mutate the input", () => {
		const params = new URLSearchParams("tab=installed");
		hubHref(params, "library");
		expect(params.toString()).toBe("tab=installed");
	});
});

describe("registry Mine states", () => {
	const packages = [
		{ id: "a", status: PackageStatus.Active },
		{ id: "b", status: PackageStatus.PendingReview },
		{ id: "c", status: PackageStatus.Disabled },
		{ id: "d", status: PackageStatus.Active },
		{ id: "e", status: PackageStatus.Rejected },
	];

	test("maps every package status", () => {
		expect(registryMineState(PackageStatus.Active)).toBe("live");
		expect(registryMineState(PackageStatus.PendingReview)).toBe("in_review");
		expect(registryMineState(PackageStatus.Rejected)).toBe("rejected");
		expect(registryMineState(PackageStatus.Disabled)).toBe("disabled");
		expect(registryMineState(PackageStatus.Yanked)).toBe("disabled");
		expect(registryMineState(PackageStatus.Deprecated)).toBe("deprecated");
		expect(registryMineState(undefined)).toBe("live");
	});

	test("counts each state", () => {
		expect(registryMineCounts(packages)).toEqual({
			all: 5,
			live: 2,
			in_review: 1,
			rejected: 1,
			disabled: 1,
			deprecated: 0,
		});
	});

	test("filters by state", () => {
		expect(filterRegistryMine(packages, "all").map((pkg) => pkg.id)).toEqual([
			"a",
			"b",
			"c",
			"d",
			"e",
		]);
		expect(filterRegistryMine(packages, "live").map((pkg) => pkg.id)).toEqual([
			"a",
			"d",
		]);
		expect(filterRegistryMine(packages, "deprecated")).toEqual([]);
	});

	test("offers chips for present states and the active one", () => {
		const counts = registryMineCounts(packages);
		expect(registryMineFilterKeys(counts, "all")).toEqual([
			"all",
			"live",
			"in_review",
			"rejected",
			"disabled",
		]);
		expect(
			registryMineFilterKeys(registryMineCounts([]), "deprecated"),
		).toEqual(["all", "deprecated"]);
	});
});

describe("sortRegistryMine", () => {
	const pkg = (id: string, status: PackageStatus, name = id) => ({
		id,
		name,
		status,
	});
	const packages = [
		pkg("zeta", PackageStatus.Active),
		pkg("old", PackageStatus.Deprecated),
		pkg("alpha", PackageStatus.Active),
		pkg("review", PackageStatus.PendingReview),
		pkg("yanked", PackageStatus.Yanked),
		pkg("nope", PackageStatus.Rejected),
		pkg("off", PackageStatus.Disabled),
	];
	const ids = (sorted: { id: string }[]) => sorted.map((entry) => entry.id);

	test("needs attention: rejected, disabled, in review, live, deprecated; ties by name", () => {
		expect(ids(sortRegistryMine(packages, "attention"))).toEqual([
			"nope",
			"off",
			"yanked",
			"review",
			"alpha",
			"zeta",
			"old",
		]);
	});

	test("name ignores case and state", () => {
		expect(
			ids(
				sortRegistryMine(
					[
						pkg("b", PackageStatus.Rejected, "beta"),
						pkg("a", PackageStatus.Active, "Alpha"),
						pkg("c", PackageStatus.Active, "charlie"),
					],
					"name",
				),
			),
		).toEqual(["a", "b", "c"]);
	});

	test("sorts by the name the card shows and breaks equal names by id", () => {
		expect(
			ids(
				sortRegistryMine(
					[
						{
							...pkg("pkg-z", PackageStatus.Active, "zzz"),
							metadata: {
								lang: "en",
								name: "Aardvark",
								description: "",
							},
						},
						pkg("pkg-b", PackageStatus.Active, "same"),
						pkg("pkg-a", PackageStatus.Active, "Same"),
					],
					"name",
				),
			),
		).toEqual(["pkg-z", "pkg-a", "pkg-b"]);
	});

	test("does not mutate the input", () => {
		const input = [...packages];
		sortRegistryMine(input, "attention");
		expect(ids(input)).toEqual(ids(packages));
	});
});
