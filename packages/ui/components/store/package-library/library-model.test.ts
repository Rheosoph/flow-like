import { describe, expect, test } from "bun:test";
import { PackagePermissionBits } from "../../../lib/permission/wasm-package-permission";
import {
	type InstalledPackage,
	PackageStatus,
	type PackageSummary,
} from "../../../lib/schema/wasm";
import { getPackageOverviewHref } from "../package-navigation";
import {
	filterLibrary,
	installedCapabilityTags,
	libraryCounts,
	libraryIdsToLookUp,
	libraryStoreHref,
	librarySummary,
	libraryUpdateCheck,
	mergeLibrary,
	resolveLibrarySignedIn,
	updateConsent,
} from "./library-model";

function summary(
	id: string,
	overrides: Partial<PackageSummary> = {},
): PackageSummary {
	return {
		id,
		name: id,
		description: `${id} package`,
		latestVersion: "1.0.0",
		downloadCount: 0,
		status: PackageStatus.Active,
		keywords: [],
		verified: false,
		price: 0,
		visibility: "public",
		...overrides,
	};
}

function installed(
	id: string,
	manifest: Record<string, unknown> = {},
	source: "remote" | "local" = "remote",
): InstalledPackage {
	return {
		id,
		version: "1.0.0",
		source: { type: source },
		installedAt: "2026-09-24T00:00:00Z",
		wasmPath: `/packages/${id}.wasm`,
		manifest: {
			id,
			name: `${id} manifest`,
			description: "",
			keywords: [],
			...manifest,
		} as unknown as InstalledPackage["manifest"],
	};
}

const MAP_WIDGET = {
	id: "live-map",
	name: "Live map",
	contract: {
		id: "live-map",
		csp: [
			{ reason: "Loads map tiles", connectSrc: ["https://api.maptiler.com"] },
		],
	},
};

const INPUT_WIDGET = {
	id: "title",
	name: "Title",
	contract: {
		id: "title",
		csp: [
			{
				reason: "Shows a title image given at runtime",
				inputs: [{ path: "title", directives: ["imgSrc"] }],
			},
		],
	},
};

const PLAIN_WIDGET = {
	id: "plain",
	name: "Plain",
	contract: {
		id: "plain",
		csp: [{ reason: "Nothing at all", connectSrc: [] }],
	},
};

describe("mergeLibrary", () => {
	const entries = mergeLibrary({
		library: [
			summary("bought", { viewerPermission: PackagePermissionBits.Buyer }),
			summary("shared", { viewerPermission: PackagePermissionBits.User }),
			summary("legacy-paid", { price: 499 }),
		],
		installed: [
			installed("shared"),
			installed("free"),
			installed("local", {}, "local"),
			installed("broken"),
			installed("paid-anon"),
			installed("mine"),
			installed("kept"),
		],
		updates: [
			{
				packageId: "shared",
				packageName: "shared",
				currentVersion: "1.0.0",
				latestVersion: "1.1.0",
			},
			{
				packageId: "local",
				packageName: "local",
				currentVersion: "1.0.0",
				latestVersion: "9.0.0",
			},
		],
		lookups: [
			summary("free", { name: "Free Tools" }),
			summary("local", { name: "Someone else's package" }),
			summary("paid-anon", { price: 900 }),
			summary("mine", {
				price: 900,
				viewerPermission: PackagePermissionBits.Owner,
			}),
			summary("kept", {
				viewerPermission: PackagePermissionBits.Maintainer,
			}),
		],
		statusOf: (id) => (id === "broken" ? "error" : undefined),
	});
	const byId = new Map(entries.map((entry) => [entry.id, entry]));

	test("lists access rows and installed packages once each, by name", () => {
		expect(entries.map((entry) => entry.id)).toEqual([
			"bought",
			"broken",
			"free",
			"kept",
			"legacy-paid",
			"local",
			"mine",
			"paid-anon",
			"shared",
		]);
	});

	test("derives access from the viewer's bits, then the listing, then the source", () => {
		expect(byId.get("bought")?.access).toBe("bought");
		expect(byId.get("shared")?.access).toBe("shared");
		expect(byId.get("legacy-paid")?.access).toBe("bought");
		expect(byId.get("free")?.access).toBe("free");
		expect(byId.get("local")?.access).toBe("local");
		expect(byId.get("mine")?.access).toBe("owner");
		expect(byId.get("kept")?.access).toBe("maintainer");
	});

	test("never calls a package free without a free registry summary", () => {
		expect(byId.get("paid-anon")?.access).toBe("unknown");
		expect(byId.get("broken")?.summary).toBeUndefined();
		expect(byId.get("broken")?.access).toBe("unknown");
	});

	test("derives the machine state", () => {
		expect(byId.get("bought")?.state).toBe("not_installed");
		expect(byId.get("shared")?.state).toBe("update");
		expect(byId.get("shared")?.update?.latestVersion).toBe("1.1.0");
		expect(byId.get("free")?.state).toBe("ready");
		expect(byId.get("broken")?.state).toBe("problem");
	});

	test("a local .wasm never takes a registry summary or update", () => {
		const local = byId.get("local");
		expect(local?.summary).toBeUndefined();
		expect(local?.update).toBeUndefined();
		expect(local?.state).toBe("ready");
		expect(local?.name).toBe("local manifest");
	});

	test("installed packages use the looked-up summary", () => {
		expect(byId.get("free")?.summary?.name).toBe("Free Tools");
	});

	test("counts every group", () => {
		expect(libraryCounts(entries)).toEqual({
			state: {
				all: 9,
				not_installed: 2,
				ready: 5,
				update: 1,
				problem: 1,
			},
			access: {
				all: 9,
				bought: 2,
				shared: 1,
				owner: 1,
				maintainer: 1,
				free: 1,
				local: 1,
			},
		});
	});

	test("filters by state and access", () => {
		expect(
			filterLibrary(entries, { state: "not_installed", access: "all" }).map(
				(entry) => entry.id,
			),
		).toEqual(["bought", "legacy-paid"]);
		expect(
			filterLibrary(entries, { state: "ready", access: "bought" }).map(
				(entry) => entry.id,
			),
		).toEqual([]);
		expect(
			filterLibrary(entries, { state: "all", access: "free" }).map(
				(entry) => entry.id,
			),
		).toEqual(["free"]);
	});
});

describe("libraryIdsToLookUp", () => {
	test("asks only for installed registry packages without an access row", () => {
		expect(
			libraryIdsToLookUp(
				[summary("bought")],
				[
					installed("zeta"),
					installed("bought"),
					installed("local", {}, "local"),
					installed("alpha"),
				],
			),
		).toEqual(["alpha", "zeta"]);
	});
});

describe("installedCapabilityTags", () => {
	test("adds widget.net for a widget with a source or a network input", () => {
		const permissions = { network: { http_enabled: true } };
		expect(
			installedCapabilityTags({ permissions, widgets: [MAP_WIDGET] }),
		).toEqual(["net.http", "widget.net"]);
		expect(installedCapabilityTags({ widgets: [INPUT_WIDGET] })).toEqual([
			"widget.net",
		]);
		expect(
			installedCapabilityTags({ permissions, widgets: [PLAIN_WIDGET] }),
		).toEqual(["net.http"]);
		expect(installedCapabilityTags(null)).toEqual([]);
	});
});

describe("updateConsent", () => {
	test("a widget.net-only escalation needs consent", () => {
		expect(
			updateConsent({
				installed: installed("maps", { widgets: [PLAIN_WIDGET] }),
				latest: ["widget.net"],
			}),
		).toEqual({ needsConsent: true, newTags: ["widget.net"] });
	});

	test("a tag present on both sides does not", () => {
		expect(
			updateConsent({
				installed: installed("maps", {
					permissions: { network: { http_enabled: true } },
					widgets: [MAP_WIDGET],
				}),
				latest: ["net.http", "widget.net"],
			}),
		).toEqual({ needsConsent: false, newTags: [] });
	});

	test("net.http newly derived from nodes needs consent", () => {
		expect(
			updateConsent({
				installed: installed("feeds", {
					permissions: { network: { http_enabled: false }, cache: true },
				}),
				latest: ["net.http", "cache"],
			}),
		).toEqual({ needsConsent: true, newTags: ["net.http"] });
	});

	test("new runtime-only tags are listed without asking", () => {
		expect(
			updateConsent({
				installed: installed("feeds"),
				latest: ["streaming", "streaming"],
			}),
		).toEqual({ needsConsent: false, newTags: ["streaming"] });
	});

	test("new file access asks, as it does everywhere hasElevatedAccess is used", () => {
		expect(
			updateConsent({
				installed: installed("feeds"),
				latest: ["storage.uploads"],
			}),
		).toEqual({ needsConsent: true, newTags: ["storage.uploads"] });
		expect(
			updateConsent({
				installed: installed("feeds", {
					permissions: { filesystem: { upload_dir: true } },
				}),
				latest: ["storage.uploads", "cache"],
			}),
		).toEqual({ needsConsent: false, newTags: ["cache"] });
	});

	test("an older registry without capabilities never asks", () => {
		expect(
			updateConsent({ installed: installed("feeds"), latest: undefined }),
		).toEqual({ needsConsent: false, newTags: [] });
	});
});

describe("libraryUpdateCheck", () => {
	const feeds = installed("feeds", {
		permissions: { network: { http_enabled: false } },
	});
	const update = {
		packageId: "feeds",
		packageName: "feeds",
		currentVersion: "1.0.0",
		latestVersion: "1.1.0",
	};
	const offered = summary("feeds", {
		latestVersion: "1.1.0",
		capabilities: ["net.http"],
	});

	test("checks the offered version against its registry summary", () => {
		expect(
			libraryUpdateCheck({ installed: feeds, update, summary: offered }, false),
		).toEqual({
			status: "verified",
			needsConsent: true,
			newTags: ["net.http"],
		});
	});

	test("waits while the summary is still loading", () => {
		expect(libraryUpdateCheck({ installed: feeds, update }, true)).toEqual({
			status: "checking",
		});
	});

	test("fails closed when the summary never came or describes another version", () => {
		expect(libraryUpdateCheck({ installed: feeds, update }, false)).toEqual({
			status: "unverified",
		});
		expect(
			libraryUpdateCheck(
				{
					installed: feeds,
					update,
					summary: { ...offered, latestVersion: "1.2.0" },
				},
				false,
			),
		).toEqual({ status: "unverified" });
	});

	test("an older registry summary without capabilities does not ask", () => {
		expect(
			libraryUpdateCheck(
				{
					installed: feeds,
					update,
					summary: { ...offered, capabilities: undefined },
				},
				false,
			),
		).toEqual({ status: "verified", needsConsent: false, newTags: [] });
	});

	test("an entry without an update has nothing to check", () => {
		expect(
			libraryUpdateCheck({ installed: feeds, summary: offered }, true),
		).toEqual({ status: "none" });
	});
});

describe("resolveLibrarySignedIn", () => {
	test("keeps a pushed sign-in through token renewals", () => {
		expect(
			resolveLibrarySignedIn(true, {
				isLoading: true,
				activeNavigator: "signinSilent",
			}),
		).toBe(true);
	});

	test("a signed-out push while OIDC is still loading is not an answer yet", () => {
		expect(
			resolveLibrarySignedIn(false, {
				isLoading: true,
				isAuthenticated: false,
			}),
		).toBeUndefined();
		expect(
			resolveLibrarySignedIn(false, {
				isLoading: false,
				activeNavigator: "signinSilent",
			}),
		).toBeUndefined();
		expect(
			resolveLibrarySignedIn(false, {
				isLoading: false,
				isAuthenticated: false,
			}),
		).toBe(false);
	});

	test("settled OIDC answers when the host never pushes", () => {
		expect(
			resolveLibrarySignedIn(undefined, {
				isLoading: false,
				isAuthenticated: false,
			}),
		).toBe(false);
		expect(
			resolveLibrarySignedIn(undefined, {
				isLoading: false,
				isAuthenticated: true,
			}),
		).toBeUndefined();
		expect(resolveLibrarySignedIn(undefined, {})).toBeUndefined();
		expect(resolveLibrarySignedIn(undefined, undefined)).toBeUndefined();
	});
});

describe("libraryStoreHref", () => {
	test("keeps the Library tab so Back from the detail returns to it", () => {
		const href = new URL(
			libraryStoreHref("com.acme/maps & more"),
			"https://flow-like.invalid",
		);
		expect(href.pathname).toBe("/store/packages");
		expect(href.searchParams.get("tab")).toBe("library");
		expect(href.searchParams.get("id")).toBe("com.acme/maps & more");
		expect(getPackageOverviewHref(href.searchParams)).toBe(
			"/store/packages?tab=library",
		);
	});
});

describe("librarySummary", () => {
	test("builds card data for a package the registry does not describe", () => {
		const [entry] = mergeLibrary({
			library: [],
			installed: [
				installed(
					"local",
					{
						description: "Local build",
						keywords: ["dev"],
						widgets: [MAP_WIDGET],
					},
					"local",
				),
			],
			updates: [],
			lookups: [],
		});
		expect(librarySummary(entry)).toMatchObject({
			id: "local",
			name: "local manifest",
			description: "Local build",
			latestVersion: "1.0.0",
			keywords: ["dev"],
			capabilities: ["widget.net"],
		});
	});
});
