import { describe, expect, test } from "bun:test";
import {
	type InstalledPackage,
	PackageStatus,
	type PackageSummary,
} from "../../lib/schema/wasm";
import {
	accessWeight,
	isTemplatePackage,
	packageInitials,
	shelfPackageFromInstalled,
	shelfPackageFromSummary,
	shelfPins,
	shelfTopics,
	sortShelf,
} from "./package-shelf-model";

const files = {
	node_storage: true,
	user_storage: true,
	upload_dir: true,
	cache_dir: true,
};

function installed(
	manifest: Record<string, unknown>,
	id = String(manifest.id),
): InstalledPackage {
	return {
		id,
		version: "0.1.0",
		source: { type: "local" },
		installedAt: "2026-09-23T14:35:44Z",
		wasmPath: "/tmp/pkg.wasm",
		manifest: manifest as unknown as InstalledPackage["manifest"],
	};
}

function summary(overrides: Partial<PackageSummary>): PackageSummary {
	return {
		id: "com.example.pkg",
		name: "Example",
		description: "An example",
		latestVersion: "1.0.0",
		downloadCount: 12,
		status: PackageStatus.Active,
		keywords: [],
		verified: false,
		price: 0,
		visibility: "public",
		...overrides,
	};
}

const typst = installed({
	id: "com.flow-like.typst",
	name: "Typst Documents",
	version: "0.2.0",
	description: "Compile Typst source into PDF, SVG, or PNG documents",
	authors: [{ name: "Felix Schultz (Rheosoph GmbH)" }, { email: "x@y.z" }],
	keywords: ["typst", "pdf", "svg", "png", "document", "template"],
	permissions: {
		memory: "heavy",
		network: { http_enabled: true, allowed_hosts: ["packages.typst.org"] },
		filesystem: files,
	},
});

const salesInsights = installed({
	id: "com.flow-like.sales-insights",
	name: "Sales Insights",
	version: "0.1.0",
	description: "Rust WASM nodes with two React micro widgets",
	keywords: ["example", "widgets", "charts"],
	permissions: { memory: "standard" },
	widgets: [
		{ id: "sales-chart", name: "Sales Chart", contract: {} },
		{ id: "filter-panel", name: "Filter Panel", contract: {} },
	],
});

const luaTemplate = installed({
	id: "com.example.lua",
	name: "Custom Node (Lua)",
	version: "0.1.0",
	description: "Template for creating Flow-Like WASM nodes in Lua",
	authors: [{ name: "Your Name" }],
	keywords: ["template", "example"],
	permissions: { streaming: true },
});

describe("shelfPackageFromInstalled", () => {
	test("reads snake_case permissions, hosts, memory and authors", () => {
		const pkg = shelfPackageFromInstalled(typst);
		expect(pkg.tags).toEqual([
			"net.http",
			"storage.user",
			"storage.node",
			"storage.uploads",
			"storage.cache",
		]);
		expect(pkg.facets).toEqual(["network", "files"]);
		expect(pkg.hosts).toEqual(["packages.typst.org"]);
		expect(pkg.memory).toBe("heavy");
		expect(pkg.authors).toEqual(["Felix Schultz (Rheosoph GmbH)"]);
		expect(pkg.template).toBe(false);
		expect(pkg.license).toBe("none");
	});

	test("counts manifest widgets and exposes them as a facet", () => {
		const pkg = shelfPackageFromInstalled(salesInsights);
		expect(pkg.widgetCount).toBe(2);
		expect(pkg.facets).toEqual(["widgets"]);
		expect(pkg.template).toBe(false);
	});

	test("keeps runtime-only permissions out of the access facets", () => {
		const pkg = shelfPackageFromInstalled(luaTemplate);
		expect(pkg.tags).toEqual(["streaming"]);
		expect(pkg.facets).toEqual([]);
		expect(pkg.template).toBe(true);
	});

	test("reads camelCase manifests and oauth providers", () => {
		const pkg = shelfPackageFromInstalled(
			installed({
				id: "com.example.mail",
				name: "Mail",
				keywords: [],
				permissions: {
					models: true,
					oauthScopes: [{ provider: "google", scopes: ["mail"] }],
					network: { httpEnabled: true },
				},
			}),
		);
		expect(pkg.tags).toEqual(["net.http", "oauth", "models"]);
		expect(pkg.facets).toEqual(["network", "accounts", "models"]);
	});

	test("survives a manifest without permissions or authors", () => {
		const pkg = shelfPackageFromInstalled(
			installed({ id: "local.node", name: "node", version: "0.0.0" }),
		);
		expect(pkg.tags).toEqual([]);
		expect(pkg.authors).toEqual([]);
		expect(pkg.keywords).toEqual([]);
		expect(pkg.description).toBe("");
	});
});

describe("shelfPackageFromSummary", () => {
	test("maps licensing the way the registry reports access", () => {
		expect(shelfPackageFromSummary(summary({})).license).toBe("none");
		expect(
			shelfPackageFromSummary(summary({ price: 900, viewerHasAccess: true }))
				.license,
		).toBe("owned");
		expect(
			shelfPackageFromSummary(
				summary({ visibility: "private", viewerHasAccess: false }),
			).license,
		).toBe("needed");
		expect(shelfPackageFromSummary(summary({ price: 900 })).license).toBe(
			"none",
		);
	});

	test("treats widget network access as network and widgets", () => {
		const pkg = shelfPackageFromSummary(
			summary({ capabilities: ["widget.net", "cache"] }),
		);
		expect(pkg.facets).toEqual(["network", "widgets"]);
	});

	test("tolerates registries that predate capabilities", () => {
		const pkg = shelfPackageFromSummary(summary({ capabilities: undefined }));
		expect(pkg.tags).toEqual([]);
		expect(pkg.facets).toEqual([]);
	});
});

describe("isTemplatePackage", () => {
	test("needs both scaffold keywords", () => {
		expect(isTemplatePackage(["template", "example"])).toBe(true);
		expect(isTemplatePackage(["Template", "Example", "typescript"])).toBe(true);
		expect(isTemplatePackage(["typst", "template"])).toBe(false);
		expect(isTemplatePackage(["example", "widgets"])).toBe(false);
	});
});

describe("sortShelf", () => {
	const pkgs = [typst, salesInsights, luaTemplate].map(
		shelfPackageFromInstalled,
	);

	test("orders by elevated access, then name", () => {
		expect(sortShelf(pkgs, "least").map((p) => p.name)).toEqual([
			"Sales Insights",
			"Custom Node (Lua)",
			"Typst Documents",
		]);
		expect(sortShelf(pkgs, "most").map((p) => p.name)).toEqual([
			"Typst Documents",
			"Custom Node (Lua)",
			"Sales Insights",
		]);
	});

	test("keeps server order for relevance and never mutates the input", () => {
		const before = pkgs.map((p) => p.name);
		expect(sortShelf(pkgs, "relevance").map((p) => p.name)).toEqual(before);
		sortShelf(pkgs, "name");
		expect(pkgs.map((p) => p.name)).toEqual(before);
	});

	test("weights network over a pile of runtime tags", () => {
		const [typstPkg, , lua] = pkgs;
		expect(accessWeight(typstPkg)).toBeGreaterThan(accessWeight(lua));
	});
});

describe("shelfTopics", () => {
	test("returns shared keywords by frequency without template markers", () => {
		const pkgs = [
			typst,
			salesInsights,
			luaTemplate,
			installed({ id: "a", name: "PDF Utils", keywords: ["pdf", "PDF"] }),
			installed({ id: "b", name: "Charts", keywords: ["charts"] }),
		].map(shelfPackageFromInstalled);
		expect(shelfTopics(pkgs)).toEqual(["charts", "pdf"]);
		expect(shelfTopics(pkgs, { min: 1, limit: 3 })).toEqual([
			"charts",
			"pdf",
			"document",
		]);
	});
});

describe("shelfPins", () => {
	test("labels a single allowed host and marks elevated access", () => {
		expect(shelfPins(shelfPackageFromInstalled(typst))).toEqual([
			{
				facet: "network",
				tags: ["net.http"],
				elevated: true,
				host: "packages.typst.org",
			},
			{
				facet: "files",
				tags: [
					"storage.user",
					"storage.node",
					"storage.uploads",
					"storage.cache",
				],
				elevated: true,
				host: undefined,
			},
		]);
	});

	test("folds overflow into a trailing pin", () => {
		const pkg = shelfPackageFromSummary(
			summary({
				capabilities: ["net.http", "oauth", "models", "storage.user"],
			}),
		);
		const pins = shelfPins(pkg);
		expect(pins.map((p) => p.facet)).toEqual(["network", "files", "more"]);
		expect(pins[2].count).toBe(2);
	});
});

describe("packageInitials", () => {
	test("uses two words, or the first two letters of one", () => {
		expect(packageInitials("Typst Documents")).toBe("TD");
		expect(packageInitials("Tor")).toBe("To");
		expect(packageInitials("Compliance & Reporting Pack")).toBe("CR");
		expect(packageInitials("")).toBe("?");
	});
});
