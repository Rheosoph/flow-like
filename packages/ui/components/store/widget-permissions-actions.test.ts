import { beforeEach, describe, expect, mock, test } from "bun:test";
import {
	type ConsentStorage,
	type MicroWidgetConsentTarget,
	evaluateMicroWidgetConsent,
	grantMicroWidgetConsent,
	muteMicroWidgetRuntime,
	readMicroWidgetConsent,
	resetMicroWidgetConsentForTests,
} from "../a2ui/micro-widget-capability-consent";
import type { WidgetPolicy } from "../a2ui/micro-widget-policy";
import {
	askAgainForWidgetRuntime,
	clearPackageWidgetPermissions,
	listWidgetConsentEntries,
	revokeAppWidgetPermissions,
	revokeWidgetPermissions,
	widgetConsentRuntimeRows,
	widgetConsentSourceRows,
} from "./widget-permissions-actions";

type ConsentModule = typeof import("../a2ui/micro-widget-capability-consent");

class MemoryStorage implements ConsentStorage {
	private readonly items = new Map<string, string>();

	get length() {
		return this.items.size;
	}

	key(index: number) {
		return [...this.items.keys()][index] ?? null;
	}

	getItem(key: string) {
		return this.items.get(key) ?? null;
	}

	setItem(key: string, value: string) {
		this.items.set(key, value);
	}

	removeItem(key: string) {
		this.items.delete(key);
	}
}

const SOURCE = "registry:hub.flow-like.com";

const MAP_POLICY: WidgetPolicy = {
	workers: true,
	csp: {
		connectSrc: ["https://api.maptiler.com"],
		imgSrc: ["https://a.tile.openstreetmap.org"],
	},
};

const CHART_POLICY: WidgetPolicy = { wasm: true };

function target(
	appId: string | null,
	packageId: string,
	widgetId: string,
): MicroWidgetConsentTarget {
	return appId === null
		? { source: SOURCE, packageId, widgetId }
		: { source: SOURCE, appId, packageId, widgetId };
}

function registry() {
	return {
		revokeWidgetGrants: mock(
			async (_packageId: string, _widgetId?: string) => {},
		),
	};
}

let storage: MemoryStorage;

beforeEach(() => {
	resetMicroWidgetConsentForTests();
	storage = new MemoryStorage();
});

function grantApp(grant: MicroWidgetConsentTarget, policy: WidgetPolicy) {
	grantMicroWidgetConsent(grant, policy, "app", storage);
}

describe("listWidgetConsentEntries", () => {
	test("filters by project, package and widget; null matches grants outside a project", () => {
		grantApp(target("app-1", "maps", "live-map"), MAP_POLICY);
		grantApp(target("app-2", "maps", "live-map"), MAP_POLICY);
		grantApp(target("app-1", "charts", "chart"), CHART_POLICY);
		grantMicroWidgetConsent(
			target(null, "maps", "live-map"),
			MAP_POLICY,
			"session",
			storage,
		);

		expect(
			listWidgetConsentEntries({ appId: "app-1" }, storage).map(
				(entry) => entry.target.widgetId,
			),
		).toEqual(["chart", "live-map"]);
		expect(
			listWidgetConsentEntries({ appId: null }, storage).map((entry) => [
				entry.target.appId ?? null,
				entry.scope,
			]),
		).toEqual([[null, "session"]]);
		expect(
			listWidgetConsentEntries({ packageId: "maps" }, storage),
		).toHaveLength(3);
		expect(
			listWidgetConsentEntries(
				{ appId: "app-1", packageId: "maps", widgetId: "live-map" },
				storage,
			),
		).toHaveLength(1);
	});
});

describe("revokeWidgetPermissions", () => {
	test("revokes the entry and keeps desktop grants while another project still allows the widget", async () => {
		grantApp(target("app-1", "maps", "live-map"), MAP_POLICY);
		grantApp(target("app-2", "maps", "live-map"), MAP_POLICY);
		const grants = registry();

		const [first] = listWidgetConsentEntries({ appId: "app-1" }, storage);
		await revokeWidgetPermissions([first], grants, storage);

		expect(
			readMicroWidgetConsent(
				target("app-1", "maps", "live-map"),
				MAP_POLICY,
				storage,
			),
		).toBe("pending");
		expect(
			readMicroWidgetConsent(
				target("app-2", "maps", "live-map"),
				MAP_POLICY,
				storage,
			),
		).toBe("granted");
		expect(grants.revokeWidgetGrants).not.toHaveBeenCalled();

		const [second] = listWidgetConsentEntries({ appId: "app-2" }, storage);
		await revokeWidgetPermissions([second], grants, storage);
		expect(grants.revokeWidgetGrants.mock.calls).toEqual([
			["maps", "live-map"],
		]);
		expect(listWidgetConsentEntries({}, storage)).toEqual([]);
	});

	test("revokes consent even when the backend cannot drop grants", async () => {
		grantApp(target("app-1", "maps", "live-map"), MAP_POLICY);
		const throwing = new Proxy(
			{},
			{
				get: () => {
					throw new Error("RegistryState is not available during prerender");
				},
			},
		);
		await revokeWidgetPermissions(
			listWidgetConsentEntries({}, storage),
			throwing,
			storage,
		);
		expect(listWidgetConsentEntries({}, storage)).toEqual([]);

		grantApp(target("app-1", "maps", "live-map"), MAP_POLICY);
		const failing = {
			revokeWidgetGrants: mock(async () => {
				throw new Error("registry offline");
			}),
		};
		await expect(
			revokeWidgetPermissions(
				listWidgetConsentEntries({}, storage),
				failing,
				storage,
			),
		).rejects.toThrow("registry offline");
		expect(listWidgetConsentEntries({}, storage)).toEqual([]);
	});
});

describe("revokeAppWidgetPermissions", () => {
	test("clears one project and drops grants only for widgets nothing else allows", async () => {
		grantApp(target("app-1", "maps", "live-map"), MAP_POLICY);
		grantApp(target("app-1", "charts", "chart"), CHART_POLICY);
		grantApp(target("app-2", "maps", "live-map"), MAP_POLICY);
		const grants = registry();

		const count = await revokeAppWidgetPermissions("app-1", grants, storage);

		expect(count).toBe(2);
		expect(listWidgetConsentEntries({ appId: "app-1" }, storage)).toEqual([]);
		expect(listWidgetConsentEntries({ appId: "app-2" }, storage)).toHaveLength(
			1,
		);
		expect(grants.revokeWidgetGrants.mock.calls).toEqual([["charts", "chart"]]);
	});
});

describe("clearPackageWidgetPermissions", () => {
	test("forgets the package in every project and drops all of its grants", async () => {
		grantApp(target("app-1", "maps", "live-map"), MAP_POLICY);
		grantApp(target("app-2", "maps", "legend"), CHART_POLICY);
		grantApp(target("app-1", "charts", "chart"), CHART_POLICY);
		const grants = registry();

		const count = await clearPackageWidgetPermissions("maps", grants, storage);

		expect(count).toBe(2);
		expect(listWidgetConsentEntries({ packageId: "maps" }, storage)).toEqual(
			[],
		);
		expect(
			readMicroWidgetConsent(
				target("app-1", "charts", "chart"),
				CHART_POLICY,
				storage,
			),
		).toBe("granted");
		expect(grants.revokeWidgetGrants.mock.calls).toEqual([["maps"]]);
	});

	test("revokes one-time grants that another tab holds for the package", async () => {
		const specifier = "../a2ui/micro-widget-capability-consent.ts?realm=tab";
		const other = (await import(specifier)) as ConsentModule;
		other.resetMicroWidgetConsentForTests();
		const mounted = target("app-1", "maps", "live-map");
		const unrelated = target("app-1", "charts", "chart");
		other.grantMicroWidgetConsent(mounted, MAP_POLICY, "session", storage);
		other.grantMicroWidgetConsent(unrelated, CHART_POLICY, "session", storage);
		const grants = registry();

		expect(await clearPackageWidgetPermissions("maps", grants, storage)).toBe(
			0,
		);

		expect(other.readMicroWidgetConsent(mounted, MAP_POLICY, storage)).toBe(
			"pending",
		);
		expect(other.readMicroWidgetConsent(unrelated, CHART_POLICY, storage)).toBe(
			"granted",
		);
		expect(grants.revokeWidgetGrants.mock.calls).toEqual([["maps"]]);

		grantMicroWidgetConsent(mounted, MAP_POLICY, "session", storage);
		expect(readMicroWidgetConsent(mounted, MAP_POLICY, storage)).toBe(
			"granted",
		);
	});

	test("still drops desktop grants when nothing was stored", async () => {
		const grants = registry();
		expect(await clearPackageWidgetPermissions("maps", grants, storage)).toBe(
			0,
		);
		expect(grants.revokeWidgetGrants.mock.calls).toEqual([["maps"]]);
	});
});

const TILE_SOURCE = "https://a.tiles.example.com";

function grantWithRuntime(grant: MicroWidgetConsentTarget) {
	grantMicroWidgetConsent(
		grant,
		{
			policy: MAP_POLICY,
			levels: {
				"https://api.maptiler.com": "external",
				"https://a.tile.openstreetmap.org": "known",
			},
			runtime: [
				{
					directive: "imgSrc",
					source: TILE_SOURCE,
					level: "external",
					slot: "tileUrl",
				},
				{
					directive: "connectSrc",
					source: TILE_SOURCE,
					level: "external",
					slot: "tileUrl",
				},
				{
					directive: "imgSrc",
					source: "https://b.tiles.example.com",
					level: "shared",
					slot: "tileUrl",
				},
			],
		},
		"app",
		storage,
	);
}

describe("consent rows", () => {
	test("declared addresses carry the level they were approved at", () => {
		grantWithRuntime(target("app-1", "maps", "live-map"));
		const [entry] = listWidgetConsentEntries({ appId: "app-1" }, storage);
		expect(widgetConsentSourceRows(entry)).toEqual([
			{
				source: "https://a.tile.openstreetmap.org",
				directives: ["imgSrc"],
				level: "known",
			},
			{
				source: "https://api.maptiler.com",
				directives: ["connectSrc"],
				level: "external",
			},
		]);
		expect(
			widgetConsentSourceRows({ policy: CHART_POLICY, levels: {} }),
		).toEqual([]);
		expect(
			widgetConsentSourceRows({
				policy: { csp: { imgSrc: ["https://old.example.com"] } },
				levels: {},
			}),
		).toEqual([{ source: "https://old.example.com", directives: ["imgSrc"] }]);
	});

	test("runtime addresses merge their directives and keep when they were approved", () => {
		grantWithRuntime(target("app-1", "maps", "live-map"));
		const [entry] = listWidgetConsentEntries({ appId: "app-1" }, storage);
		const rows = widgetConsentRuntimeRows(entry);
		expect(
			rows.map(({ source, directives, level }) => [source, directives, level]),
		).toEqual([
			["https://a.tiles.example.com", ["connectSrc", "imgSrc"], "external"],
			["https://b.tiles.example.com", ["imgSrc"], "shared"],
		]);
		expect(rows.every((row) => row.at === entry.grantedAt)).toBe(true);
	});

	test("orders runtime addresses newest first", () => {
		expect(
			widgetConsentRuntimeRows({
				runtime: [
					{ d: "imgSrc", s: "https://old.example.com", l: "known", at: 1 },
					{ d: "imgSrc", s: "https://new.example.com", l: "broad", at: 5 },
					{
						d: "connectSrc",
						s: "https://old.example.com",
						l: "external",
						at: 3,
					},
				],
			}),
		).toEqual([
			{
				source: "https://new.example.com",
				directives: ["imgSrc"],
				level: "broad",
				at: 5,
			},
			{
				source: "https://old.example.com",
				directives: ["connectSrc", "imgSrc"],
				level: "external",
				at: 3,
			},
		]);
	});
});

describe("askAgainForWidgetRuntime", () => {
	test("clears stop asking and keeps what was allowed", () => {
		const mounted = target("app-1", "maps", "live-map");
		grantWithRuntime(mounted);
		muteMicroWidgetRuntime(mounted, storage);
		const [muted] = listWidgetConsentEntries({ appId: "app-1" }, storage);
		expect(muted.runtimeMuted).toBe(true);
		expect(evaluateMicroWidgetConsent(mounted, MAP_POLICY, storage).muted).toBe(
			true,
		);

		askAgainForWidgetRuntime(muted, storage);

		const [entry] = listWidgetConsentEntries({ appId: "app-1" }, storage);
		expect(entry.runtimeMuted).toBe(false);
		expect(widgetConsentRuntimeRows(entry)).toHaveLength(2);
		expect(evaluateMicroWidgetConsent(mounted, MAP_POLICY, storage).muted).toBe(
			false,
		);
	});

	test("forgets a record that only said stop asking", () => {
		const mounted = target("app-1", "maps", "live-map");
		muteMicroWidgetRuntime(mounted, storage);
		const entries = listWidgetConsentEntries({ appId: "app-1" }, storage);
		expect(entries.map((entry) => entry.runtimeMuted)).toEqual([true]);

		askAgainForWidgetRuntime(entries[0], storage);

		expect(listWidgetConsentEntries({ appId: "app-1" }, storage)).toEqual([]);
	});
});
