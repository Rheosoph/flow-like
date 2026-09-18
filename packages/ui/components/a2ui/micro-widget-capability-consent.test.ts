import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import type { WidgetContract } from "@flow-like/widget-sdk";
import {
	type ConsentStorage,
	MICRO_WIDGET_RUNTIME_CONSENT_LIMIT,
	MICRO_WIDGET_RUNTIME_REFRESH_MS,
	type MicroWidgetConsentRequest,
	type MicroWidgetConsentTarget,
	blockMicroWidgetConsent,
	blockMicroWidgetRuntimeSources,
	clearMicroWidgetConsents,
	clearPackageMicroWidgetConsents,
	evaluateMicroWidgetConsent,
	grantMicroWidgetConsent,
	listMicroWidgetConsents,
	microWidgetConsentKey,
	microWidgetConsentRevokedKey,
	muteMicroWidgetRuntime,
	readMicroWidgetConsent,
	readMicroWidgetRuntimeBlocks,
	readRequestedCapabilities,
	refreshMicroWidgetRuntimeConsent,
	reopenMicroWidgetConsent,
	resetMicroWidgetConsentForTests,
	revokeMicroWidgetConsent,
	subscribeMicroWidgetConsent,
	unmuteMicroWidgetRuntime,
} from "./micro-widget-capability-consent";
import type {
	WidgetPolicy,
	WidgetRuntimeSourceEntry,
	WidgetSourceLevel,
} from "./micro-widget-policy";

type ConsentModule = typeof import("./micro-widget-capability-consent");

const contract = (capabilities: unknown): WidgetContract =>
	({ contractVersion: 1, id: "chart", capabilities }) as WidgetContract;

const target: MicroWidgetConsentTarget = {
	source: "registry:hub.flow-like.com",
	appId: "app1",
	packageId: "com.example.maps",
	widgetId: "live-map",
};

const network: WidgetPolicy = {
	workers: true,
	csp: {
		connectSrc: ["https://api.maptiler.com"],
		imgSrc: ["https://a.tile.openstreetmap.org"],
	},
};

class FakeStorage implements ConsentStorage {
	readonly items = new Map<string, string>();
	failing = false;
	get length() {
		if (this.failing) throw new Error("blocked");
		return this.items.size;
	}
	key(index: number) {
		if (this.failing) throw new Error("blocked");
		return [...this.items.keys()][index] ?? null;
	}
	getItem(key: string) {
		if (this.failing) throw new Error("blocked");
		return this.items.get(key) ?? null;
	}
	setItem(key: string, value: string) {
		if (this.failing) throw new Error("blocked");
		this.items.set(key, value);
	}
	removeItem(key: string) {
		if (this.failing) throw new Error("blocked");
		this.items.delete(key);
	}
}

/** Another tab: its own session state, the same device storage. */
async function otherRealm(): Promise<ConsentModule> {
	const specifier = "./micro-widget-capability-consent.ts?realm=other";
	return (await import(specifier)) as ConsentModule;
}

let storage: FakeStorage;

beforeEach(() => {
	resetMicroWidgetConsentForTests();
	storage = new FakeStorage();
});

describe("readRequestedCapabilities", () => {
	test("only own boolean true keys count, and previews drop audio", () => {
		const capabilities = Object.create({ workers: true }) as Record<
			string,
			unknown
		>;
		capabilities.downloads = true;
		capabilities.microphone = "true";
		capabilities.media = true;
		expect(readRequestedCapabilities(contract(capabilities), false)).toEqual([
			"downloads",
			"media",
		]);
		expect(readRequestedCapabilities(contract(capabilities), true)).toEqual([
			"downloads",
		]);
		expect(readRequestedCapabilities(contract(["wasm"]), false)).toEqual([]);
		expect(readRequestedCapabilities(null, false)).toEqual([]);
	});
});

describe("consent keys", () => {
	test("ids containing separators never share a key", () => {
		const left = microWidgetConsentKey({
			source: "local",
			appId: "a-b",
			packageId: "c",
			widgetId: "w",
		});
		const right = microWidgetConsentKey({
			source: "local",
			appId: "a",
			packageId: "b-c",
			widgetId: "w",
		});
		expect(left).not.toBe(right);
		expect(left).toBe('widget-consent:v2:["local","a-b","c","w"]');
		expect(
			microWidgetConsentKey({ source: "local", packageId: "p", widgetId: "w" }),
		).toBe('widget-consent:v2:["local",null,"p","w"]');
	});

	test("a grant for one package never covers a package whose id shifts the separator", () => {
		grantMicroWidgetConsent(
			{ source: "local", appId: "a-b", packageId: "c", widgetId: "w" },
			network,
			"app",
			storage,
		);
		resetMicroWidgetConsentForTests();
		expect(
			readMicroWidgetConsent(
				{ source: "local", appId: "a", packageId: "b-c", widgetId: "w" },
				network,
				storage,
			),
		).toBe("pending");
	});

	test("grants are bound to the source", () => {
		grantMicroWidgetConsent(target, network, "app", storage);
		expect(
			readMicroWidgetConsent({ ...target, source: "local" }, network, storage),
		).toBe("pending");
		expect(
			readMicroWidgetConsent(
				{ ...target, source: "registry:evil.example.com" },
				network,
				storage,
			),
		).toBe("pending");
	});
});

describe("coverage", () => {
	test("an empty policy is granted without a prompt", () => {
		expect(readMicroWidgetConsent(target, {}, storage)).toBe("granted");
	});

	test("a new host or capability re-prompts, removals do not", () => {
		grantMicroWidgetConsent(target, network, "session", storage);
		expect(readMicroWidgetConsent(target, network, storage)).toBe("granted");
		expect(
			readMicroWidgetConsent(
				target,
				{ csp: { connectSrc: ["https://api.maptiler.com"] } },
				storage,
			),
		).toBe("granted");
		expect(
			readMicroWidgetConsent(
				target,
				{
					...network,
					csp: {
						...network.csp,
						connectSrc: [
							"https://api.maptiler.com",
							"https://evil.example.com",
						],
					},
				},
				storage,
			),
		).toBe("pending");
		expect(
			readMicroWidgetConsent(target, { ...network, wasm: true }, storage),
		).toBe("pending");
	});

	test("a host granted for one directive does not cover another directive", () => {
		grantMicroWidgetConsent(
			target,
			{ csp: { imgSrc: ["https://api.maptiler.com"] } },
			"session",
			storage,
		);
		expect(
			readMicroWidgetConsent(
				target,
				{ csp: { connectSrc: ["https://api.maptiler.com"] } },
				storage,
			),
		).toBe("pending");
	});

	test("capability grants never cover network sources", () => {
		grantMicroWidgetConsent(target, { workers: true }, "app", storage);
		expect(readMicroWidgetConsent(target, network, storage)).toBe("pending");
	});
});

describe("scopes", () => {
	test("session grants stay in memory and notify subscribers", () => {
		let notified = 0;
		const unsubscribe = subscribeMicroWidgetConsent(() => notified++);
		grantMicroWidgetConsent(target, network, "session", storage);
		expect(notified).toBe(1);
		expect(readMicroWidgetConsent(target, network, storage)).toBe("granted");
		expect(storage.items.size).toBe(0);
		resetMicroWidgetConsentForTests();
		expect(readMicroWidgetConsent(target, network, storage)).toBe("pending");
		unsubscribe();
	});

	test("project grants store exactly the current policy", () => {
		grantMicroWidgetConsent(
			target,
			{
				...network,
				csp: {
					connectSrc: ["https://old.example.com", "https://api.maptiler.com"],
				},
			},
			"app",
			storage,
		);
		grantMicroWidgetConsent(target, network, "app", storage);
		resetMicroWidgetConsentForTests();
		const record = JSON.parse(
			storage.items.get(microWidgetConsentKey(target)) ?? "null",
		);
		expect(record.v).toBe(2);
		expect(record.policy).toEqual(network);
		expect(typeof record.grantedAt).toBe("number");
		expect(readMicroWidgetConsent(target, network, storage)).toBe("granted");
		expect(
			readMicroWidgetConsent(
				target,
				{ csp: { connectSrc: ["https://old.example.com"] } },
				storage,
			),
		).toBe("pending");
		expect(
			readMicroWidgetConsent({ ...target, appId: "app2" }, network, storage),
		).toBe("pending");
	});

	test("without an app there is no project scope", () => {
		const detached = { ...target, appId: undefined };
		grantMicroWidgetConsent(detached, network, "app", storage);
		expect(storage.items.size).toBe(0);
		expect(readMicroWidgetConsent(detached, network, storage)).toBe("granted");
	});

	test("storage failures and garbage never grant or crash", () => {
		storage.items.set(
			microWidgetConsentKey(target),
			JSON.stringify({
				v: 2,
				policy: { ...network, scriptSrc: ["https://evil.example.com"] },
				grantedAt: 1,
			}),
		);
		expect(readMicroWidgetConsent(target, network, storage)).toBe("pending");
		storage.items.set(microWidgetConsentKey(target), "{not json");
		expect(readMicroWidgetConsent(target, network, storage)).toBe("pending");
		storage.failing = true;
		expect(() =>
			grantMicroWidgetConsent(target, network, "app", storage),
		).not.toThrow();
		expect(readMicroWidgetConsent(target, network, storage)).toBe("granted");
		expect(listMicroWidgetConsents(undefined, storage)).toHaveLength(1);
		expect(() => revokeMicroWidgetConsent(target, storage)).not.toThrow();
		expect(readMicroWidgetConsent(target, network, storage)).toBe("pending");
	});
});

describe("block and revoke", () => {
	test("blocking after a grant clears it and keeps the widget blocked", () => {
		grantMicroWidgetConsent(target, network, "app", storage);
		blockMicroWidgetConsent(target, storage);
		expect(storage.items.has(microWidgetConsentKey(target))).toBe(false);
		expect(readMicroWidgetConsent(target, network, storage)).toBe("blocked");
		resetMicroWidgetConsentForTests();
		expect(readMicroWidgetConsent(target, network, storage)).toBe("pending");
	});

	test("reopening a blocked prompt and allowing again grants", () => {
		blockMicroWidgetConsent(target, storage);
		reopenMicroWidgetConsent(target);
		expect(readMicroWidgetConsent(target, network, storage)).toBe("pending");
		grantMicroWidgetConsent(target, network, "session", storage);
		expect(readMicroWidgetConsent(target, network, storage)).toBe("granted");
	});

	test("revoking forgets the decision and the next mount prompts", () => {
		grantMicroWidgetConsent(target, network, "app", storage);
		blockMicroWidgetConsent({ ...target, widgetId: "other" }, storage);
		revokeMicroWidgetConsent(target, storage);
		revokeMicroWidgetConsent({ ...target, widgetId: "other" }, storage);
		expect(readMicroWidgetConsent(target, network, storage)).toBe("pending");
		expect(
			readMicroWidgetConsent(
				{ ...target, widgetId: "other" },
				network,
				storage,
			),
		).toBe("pending");
		expect(
			Number(storage.items.get(microWidgetConsentRevokedKey(target))),
		).toBeGreaterThan(0);
	});

	test("a grant given right after a revocation counts", () => {
		revokeMicroWidgetConsent(target, storage);
		grantMicroWidgetConsent(target, network, "session", storage);
		expect(readMicroWidgetConsent(target, network, storage)).toBe("granted");
		revokeMicroWidgetConsent(target, storage);
		expect(readMicroWidgetConsent(target, network, storage)).toBe("pending");
	});

	test("a revocation in another tab invalidates this tab's session grant", async () => {
		const other = await otherRealm();
		other.resetMicroWidgetConsentForTests();
		grantMicroWidgetConsent(target, network, "session", storage);
		expect(other.readMicroWidgetConsent(target, network, storage)).toBe(
			"pending",
		);
		other.revokeMicroWidgetConsent(target, storage);
		expect(readMicroWidgetConsent(target, network, storage)).toBe("pending");

		other.grantMicroWidgetConsent(target, network, "session", storage);
		grantMicroWidgetConsent(target, network, "session", storage);
		other.blockMicroWidgetConsent(target, storage);
		expect(readMicroWidgetConsent(target, network, storage)).toBe("pending");
		expect(other.readMicroWidgetConsent(target, network, storage)).toBe(
			"blocked",
		);
	});

	test("a project grant from another tab lifts this tab's block", async () => {
		const other = await otherRealm();
		other.resetMicroWidgetConsentForTests();
		blockMicroWidgetConsent(target, storage);
		other.grantMicroWidgetConsent(target, network, "app", storage);
		expect(readMicroWidgetConsent(target, network, storage)).toBe("granted");
	});
});

describe("manage and clear", () => {
	const second: MicroWidgetConsentTarget = {
		...target,
		appId: "app2",
		widgetId: "chart",
	};

	test("lists stored and session grants per app", () => {
		grantMicroWidgetConsent(target, network, "app", storage);
		grantMicroWidgetConsent(second, { downloads: true }, "session", storage);
		const all = listMicroWidgetConsents(undefined, storage);
		expect(
			all.map(({ target: entry, scope, legacy }) => [
				entry.appId,
				entry.widgetId,
				scope,
				legacy,
			]),
		).toEqual([
			["app1", "live-map", "app", false],
			["app2", "chart", "session", false],
		]);
		expect(all[0].policy).toEqual(network);
		expect(listMicroWidgetConsents("app2", storage)).toHaveLength(1);
		revokeMicroWidgetConsent(target, storage);
		expect(listMicroWidgetConsents("app1", storage)).toEqual([]);
	});

	test("a one-time grant broader than the stored project grant is listed next to it", () => {
		const stored: WidgetPolicy = {
			csp: { connectSrc: ["https://a.example.com"] },
		};
		const broader: WidgetPolicy = {
			csp: { connectSrc: ["https://a.example.com", "https://b.example.com"] },
		};
		grantMicroWidgetConsent(target, stored, "app", storage);
		grantMicroWidgetConsent(target, stored, "session", storage);
		expect(
			listMicroWidgetConsents("app1", storage).map((entry) => entry.scope),
		).toEqual(["app"]);

		grantMicroWidgetConsent(target, broader, "session", storage);
		const [project, session, ...rest] = listMicroWidgetConsents(
			"app1",
			storage,
		);
		expect(rest).toEqual([]);
		expect([project.scope, project.policy]).toEqual(["app", stored]);
		expect([session.scope, session.policy]).toEqual(["session", broader]);
		expect(readMicroWidgetConsent(target, broader, storage)).toBe("granted");

		revokeMicroWidgetConsent(target, storage);
		expect(listMicroWidgetConsents("app1", storage)).toEqual([]);
	});

	test("clearing one app leaves other apps alone", () => {
		grantMicroWidgetConsent(target, network, "app", storage);
		grantMicroWidgetConsent(second, network, "app", storage);
		clearMicroWidgetConsents({ appId: "app1" }, storage);
		expect(readMicroWidgetConsent(target, network, storage)).toBe("pending");
		expect(readMicroWidgetConsent(second, network, storage)).toBe("granted");
		expect(
			listMicroWidgetConsents(undefined, storage).map(
				(entry) => entry.target.appId,
			),
		).toEqual(["app2"]);
	});

	test("clearing the device revokes session grants of other tabs", async () => {
		const other = await otherRealm();
		other.resetMicroWidgetConsentForTests();
		other.grantMicroWidgetConsent(second, network, "session", storage);
		grantMicroWidgetConsent(target, network, "app", storage);
		clearMicroWidgetConsents({}, storage);
		expect(readMicroWidgetConsent(target, network, storage)).toBe("pending");
		expect(other.readMicroWidgetConsent(second, network, storage)).toBe(
			"pending",
		);
		expect(
			[...storage.items.keys()].filter((key) => !key.includes("revoked")),
		).toEqual([]);
		grantMicroWidgetConsent(target, network, "app", storage);
		expect(readMicroWidgetConsent(target, network, storage)).toBe("granted");
	});

	test("clearing a package revokes its session grants in other tabs and nothing else", async () => {
		const other = await otherRealm();
		other.resetMicroWidgetConsentForTests();
		const legacyKey =
			"widget-capability-consent-app-app1-com.example.maps/live-map";
		const detached: MicroWidgetConsentTarget = {
			source: "local",
			packageId: target.packageId,
			widgetId: "legend",
		};
		const unrelated: MicroWidgetConsentTarget = {
			...target,
			packageId: "com.example.charts",
		};
		other.grantMicroWidgetConsent(target, network, "session", storage);
		other.grantMicroWidgetConsent(detached, network, "session", storage);
		other.grantMicroWidgetConsent(unrelated, network, "session", storage);
		grantMicroWidgetConsent(second, network, "app", storage);
		grantMicroWidgetConsent(
			{ ...unrelated, appId: "app2" },
			network,
			"app",
			storage,
		);
		blockMicroWidgetConsent({ ...target, widgetId: "blocked" }, storage);
		storage.items.set(legacyKey, JSON.stringify(["workers"]));

		clearPackageMicroWidgetConsents(target.packageId, storage);

		expect(other.readMicroWidgetConsent(target, network, storage)).toBe(
			"pending",
		);
		expect(other.readMicroWidgetConsent(detached, network, storage)).toBe(
			"pending",
		);
		expect(readMicroWidgetConsent(second, network, storage)).toBe("pending");
		expect(readMicroWidgetConsent(target, { workers: true }, storage)).toBe(
			"pending",
		);
		expect(
			readMicroWidgetConsent(
				{ ...target, widgetId: "blocked" },
				network,
				storage,
			),
		).toBe("pending");
		expect(other.listMicroWidgetConsents(undefined, storage)).toEqual([
			expect.objectContaining({ target: unrelated, scope: "session" }),
			expect.objectContaining({
				target: { ...unrelated, appId: "app2" },
				scope: "app",
			}),
		]);
		expect(other.readMicroWidgetConsent(unrelated, network, storage)).toBe(
			"granted",
		);
		expect(
			[...storage.items.keys()].filter((key) => !key.includes("revoked")),
		).toEqual([microWidgetConsentKey({ ...unrelated, appId: "app2" })]);

		other.grantMicroWidgetConsent(target, network, "session", storage);
		expect(other.readMicroWidgetConsent(target, network, storage)).toBe(
			"granted",
		);
	});
});

describe("legacy capability grants", () => {
	const legacyKey =
		"widget-capability-consent-app-app1-com.example.maps/live-map";

	test("cover unchanged capability widgets from any registry, never network sources", () => {
		storage.items.set(legacyKey, JSON.stringify(["workers", "downloads"]));
		expect(readMicroWidgetConsent(target, { workers: true }, storage)).toBe(
			"granted",
		);
		expect(
			readMicroWidgetConsent(
				{ ...target, source: "registry:other.example.com" },
				{ downloads: true },
				storage,
			),
		).toBe("granted");
		expect(readMicroWidgetConsent(target, network, storage)).toBe("pending");
		expect(
			readMicroWidgetConsent(
				{ ...target, source: "local" },
				{ workers: true },
				storage,
			),
		).toBe("pending");
		expect(readMicroWidgetConsent(target, { microphone: true }, storage)).toBe(
			"pending",
		);
	});

	test("ambiguous legacy keys are never read", () => {
		storage.items.set(
			"widget-capability-consent-app-a-b-c/w",
			JSON.stringify(["downloads"]),
		);
		for (const ambiguous of [
			{ appId: "a", packageId: "b-c" },
			{ appId: "a-b", packageId: "c" },
		]) {
			expect(
				readMicroWidgetConsent(
					{ ...target, ...ambiguous, widgetId: "w" },
					{ downloads: true },
					storage,
				),
			).toBe("pending");
		}
		expect(listMicroWidgetConsents(undefined, storage)).toEqual([]);
	});

	test("are listed, replaced by a project grant and removed by revoke", () => {
		storage.items.set(legacyKey, JSON.stringify(["workers"]));
		expect(listMicroWidgetConsents("app1", storage)).toEqual([
			{
				target: {
					source: "registry:*",
					appId: "app1",
					packageId: "com.example.maps",
					widgetId: "live-map",
				},
				policy: { workers: true },
				levels: {},
				runtime: [],
				runtimeMuted: false,
				grantedAt: 0,
				scope: "app",
				legacy: true,
			},
		]);
		grantMicroWidgetConsent(target, { downloads: true }, "app", storage);
		expect(storage.items.has(legacyKey)).toBe(false);
		resetMicroWidgetConsentForTests();
		expect(readMicroWidgetConsent(target, { workers: true }, storage)).toBe(
			"pending",
		);

		storage.items.set(legacyKey, JSON.stringify(["workers"]));
		revokeMicroWidgetConsent(target, storage);
		expect(storage.items.has(legacyKey)).toBe(false);
		storage.items.set(legacyKey, JSON.stringify(["workers"]));
		expect(readMicroWidgetConsent(target, { workers: true }, storage)).toBe(
			"pending",
		);
	});
});

describe("cross-tab sync", () => {
	let previousWindow: PropertyDescriptor | undefined;
	let fakeWindow: EventTarget;

	beforeEach(() => {
		previousWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
		fakeWindow = new EventTarget();
		Object.defineProperty(globalThis, "window", {
			value: fakeWindow,
			configurable: true,
			writable: true,
		});
	});

	afterEach(() => {
		if (previousWindow) {
			Object.defineProperty(globalThis, "window", previousWindow);
		} else {
			Reflect.deleteProperty(globalThis, "window");
		}
	});

	function dispatch(type: string, fields: Record<string, unknown>) {
		const event = new Event(type);
		for (const [key, value] of Object.entries(fields)) {
			Object.defineProperty(event, key, { value });
		}
		fakeWindow.dispatchEvent(event);
	}

	test("storage writes to consent keys and restored pages notify subscribers", () => {
		let notified = 0;
		const unsubscribe = subscribeMicroWidgetConsent(() => notified++);
		dispatch("storage", { key: microWidgetConsentRevokedKey(target) });
		expect(notified).toBe(1);
		dispatch("storage", { key: "widget-capability-consent-app-x-y/z" });
		expect(notified).toBe(2);
		dispatch("storage", { key: null });
		expect(notified).toBe(3);
		dispatch("storage", { key: "theme" });
		expect(notified).toBe(3);
		dispatch("pageshow", { persisted: false });
		expect(notified).toBe(3);
		dispatch("pageshow", { persisted: true });
		expect(notified).toBe(4);

		unsubscribe();
		dispatch("storage", { key: null });
		dispatch("pageshow", { persisted: true });
		expect(notified).toBe(4);
	});
});

const TILES = "https://a.tiles.customer-maps.com";
const OTHER_TILES = "https://b.tiles.customer-maps.com";

function runtime(
	source: string,
	level: WidgetSourceLevel = "external",
	directive: WidgetRuntimeSourceEntry["directive"] = "imgSrc",
): WidgetRuntimeSourceEntry {
	return { directive, source, level, slot: "tileUrl" };
}

function request(
	policy: WidgetPolicy,
	levels: Record<string, WidgetSourceLevel>,
	entries: WidgetRuntimeSourceEntry[] = [],
): MicroWidgetConsentRequest {
	return { policy, levels, runtime: entries };
}

const declared = request(network, {
	"https://api.maptiler.com": "shared",
	"https://a.tile.openstreetmap.org": "known",
});

function storedRecord(key = target) {
	return JSON.parse(storage.items.get(microWidgetConsentKey(key)) ?? "null");
}

describe("levels", () => {
	test("a raised level re-prompts and is reported, a lowered one does not", () => {
		grantMicroWidgetConsent(target, declared, "app", storage);
		expect(readMicroWidgetConsent(target, declared, storage)).toBe("granted");
		const lowered = request(network, {
			"https://api.maptiler.com": "external",
			"https://a.tile.openstreetmap.org": "known",
		});
		expect(readMicroWidgetConsent(target, lowered, storage)).toBe("granted");
		const raised = request(network, {
			"https://api.maptiler.com": "broad",
			"https://a.tile.openstreetmap.org": "known",
		});
		const evaluation = evaluateMicroWidgetConsent(target, raised, storage);
		expect(evaluation.status).toBe("pending");
		expect(evaluation.declaredCovered).toBe(false);
		expect(evaluation.raisedSources).toEqual(["https://api.maptiler.com"]);
		expect(evaluation.newSources).toEqual([]);
	});

	test("the project record stores the level of every declared source", () => {
		grantMicroWidgetConsent(target, declared, "app", storage);
		expect(storedRecord()).toMatchObject({
			v: 2,
			policy: network,
			levels: {
				"https://api.maptiler.com": "shared",
				"https://a.tile.openstreetmap.org": "known",
			},
		});
		expect(storedRecord().runtime).toBeUndefined();
		expect(storedRecord().runtimeMuted).toBeUndefined();
	});

	test("a bare policy asks for and grants the worst level", () => {
		grantMicroWidgetConsent(target, network, "session", storage);
		expect(readMicroWidgetConsent(target, declared, storage)).toBe("granted");
		resetMicroWidgetConsentForTests();
		grantMicroWidgetConsent(target, declared, "session", storage);
		expect(readMicroWidgetConsent(target, network, storage)).toBe("pending");
	});

	test("records from before levels re-prompt for sources once, capabilities stay covered", () => {
		storage.items.set(
			microWidgetConsentKey(target),
			JSON.stringify({ v: 2, policy: network, grantedAt: 1 }),
		);
		expect(readMicroWidgetConsent(target, { workers: true }, storage)).toBe(
			"granted",
		);
		const evaluation = evaluateMicroWidgetConsent(target, declared, storage);
		expect(evaluation.status).toBe("pending");
		expect(evaluation.raisedSources).toEqual([]);
		expect(evaluation.newSources).toEqual([]);
		grantMicroWidgetConsent(target, declared, "app", storage);
		resetMicroWidgetConsentForTests();
		expect(readMicroWidgetConsent(target, declared, storage)).toBe("granted");
	});

	test("a malformed level leaves only its source uncovered and malformed runtime is ignored alone", () => {
		storage.items.set(
			microWidgetConsentKey(target),
			JSON.stringify({
				v: 2,
				policy: network,
				levels: {
					"https://api.maptiler.com": "unknown",
					"https://a.tile.openstreetmap.org": "known",
				},
				runtime: "garbage",
				grantedAt: 1,
			}),
		);
		expect(
			readMicroWidgetConsent(
				target,
				request(
					{ csp: { imgSrc: ["https://a.tile.openstreetmap.org"] } },
					{ "https://a.tile.openstreetmap.org": "known" },
				),
				storage,
			),
		).toBe("granted");
		expect(readMicroWidgetConsent(target, declared, storage)).toBe("pending");
		storage.items.set(
			microWidgetConsentKey(target),
			JSON.stringify({
				v: 2,
				policy: {},
				levels: {},
				runtime: [
					{ d: "imgSrc", s: TILES, l: "external", at: 5 },
					{ d: "frameSrc", s: OTHER_TILES, l: "external", at: 5 },
					{ d: "imgSrc", s: OTHER_TILES, l: "nope", at: 5 },
				],
				grantedAt: 1,
			}),
		);
		expect(
			evaluateMicroWidgetConsent(
				target,
				request({}, {}, [runtime(TILES), runtime(OTHER_TILES)]),
				storage,
			).uncoveredRuntime.map((entry) => entry.source),
		).toEqual([OTHER_TILES]);
	});
});

describe("runtime sources", () => {
	const withRuntime = (...entries: WidgetRuntimeSourceEntry[]) =>
		request(declared.policy, declared.levels, entries);

	test("an approval covers the same directive, source and level or lower", () => {
		grantMicroWidgetConsent(
			target,
			withRuntime(runtime(TILES)),
			"app",
			storage,
		);
		resetMicroWidgetConsentForTests();
		expect(
			readMicroWidgetConsent(target, withRuntime(runtime(TILES)), storage),
		).toBe("granted");
		expect(
			readMicroWidgetConsent(
				target,
				withRuntime(runtime(TILES, "known")),
				storage,
			),
		).toBe("granted");
		const raised = evaluateMicroWidgetConsent(
			target,
			withRuntime(runtime(TILES, "broad")),
			storage,
		);
		expect(raised.status).toBe("pending");
		expect(raised.declaredCovered).toBe(true);
		expect(raised.raisedSources).toEqual([TILES]);
		expect(
			readMicroWidgetConsent(
				target,
				withRuntime(runtime(TILES, "external", "connectSrc")),
				storage,
			),
		).toBe("pending");
		const fresh = evaluateMicroWidgetConsent(
			target,
			withRuntime(runtime(OTHER_TILES)),
			storage,
		);
		expect(fresh.uncoveredRuntime.map((entry) => entry.source)).toEqual([
			OTHER_TILES,
		]);
		expect(fresh.newSources).toEqual([OTHER_TILES]);
	});

	test("runtime approvals never cover a declared source", () => {
		grantMicroWidgetConsent(
			target,
			request({}, {}, [
				runtime("https://api.maptiler.com", "known", "connectSrc"),
			]),
			"session",
			storage,
		);
		expect(
			readMicroWidgetConsent(
				target,
				request(
					{ csp: { connectSrc: ["https://api.maptiler.com"] } },
					{ "https://api.maptiler.com": "known" },
				),
				storage,
			),
		).toBe("pending");
	});

	test("a declared grant covers the same source when it arrives at runtime", () => {
		grantMicroWidgetConsent(target, declared, "session", storage);
		expect(
			readMicroWidgetConsent(
				target,
				withRuntime(runtime("https://a.tile.openstreetmap.org", "known")),
				storage,
			),
		).toBe("granted");
	});

	test("approvals on different pages add up without re-prompting", () => {
		grantMicroWidgetConsent(
			target,
			withRuntime(runtime(TILES)),
			"app",
			storage,
		);
		grantMicroWidgetConsent(
			target,
			withRuntime(runtime(OTHER_TILES)),
			"app",
			storage,
		);
		resetMicroWidgetConsentForTests();
		expect(
			readMicroWidgetConsent(target, withRuntime(runtime(TILES)), storage),
		).toBe("granted");
		expect(
			readMicroWidgetConsent(
				target,
				withRuntime(runtime(TILES), runtime(OTHER_TILES)),
				storage,
			),
		).toBe("granted");
		expect(
			storedRecord()
				.runtime.map((entry: { s: string }) => entry.s)
				.sort(),
		).toEqual([TILES, OTHER_TILES]);
	});

	test("allow once keeps runtime approvals out of the project record", () => {
		grantMicroWidgetConsent(
			target,
			withRuntime(runtime(TILES)),
			"app",
			storage,
		);
		grantMicroWidgetConsent(
			target,
			withRuntime(runtime(OTHER_TILES)),
			"session",
			storage,
		);
		expect(
			readMicroWidgetConsent(
				target,
				withRuntime(runtime(OTHER_TILES)),
				storage,
			),
		).toBe("granted");
		expect(
			storedRecord().runtime.map((entry: { s: string }) => entry.s),
		).toEqual([TILES]);
		resetMicroWidgetConsentForTests();
		expect(
			readMicroWidgetConsent(
				target,
				withRuntime(runtime(OTHER_TILES)),
				storage,
			),
		).toBe("pending");
	});

	test("the record keeps the most recent runtime approvals", () => {
		const entries = Array.from(
			{ length: MICRO_WIDGET_RUNTIME_CONSENT_LIMIT + 6 },
			(_, index) => ({
				d: "imgSrc",
				s: `https://h${index}.example.com`,
				l: "external",
				at: 1000 + index,
			}),
		);
		storage.items.set(
			microWidgetConsentKey(target),
			JSON.stringify({
				v: 2,
				policy: {},
				levels: {},
				runtime: entries,
				grantedAt: 1,
			}),
		);
		grantMicroWidgetConsent(
			target,
			withRuntime(runtime(TILES)),
			"app",
			storage,
		);
		const kept = storedRecord().runtime as { s: string; at: number }[];
		expect(kept).toHaveLength(MICRO_WIDGET_RUNTIME_CONSENT_LIMIT);
		expect(kept[0].s).toBe(TILES);
		expect(kept.map((entry) => entry.s)).not.toContain(
			"https://h6.example.com",
		);
		expect(kept.map((entry) => entry.s)).toContain("https://h7.example.com");
	});

	test("a revocation in another tab stays in force for entries a stale tab still holds", async () => {
		const other = await otherRealm();
		other.resetMicroWidgetConsentForTests();
		grantMicroWidgetConsent(
			target,
			withRuntime(runtime(TILES)),
			"session",
			storage,
		);
		other.revokeMicroWidgetConsent(target, storage);
		grantMicroWidgetConsent(
			target,
			withRuntime(runtime(OTHER_TILES)),
			"session",
			storage,
		);
		expect(
			readMicroWidgetConsent(
				target,
				withRuntime(runtime(OTHER_TILES)),
				storage,
			),
		).toBe("granted");
		const stale = evaluateMicroWidgetConsent(
			target,
			withRuntime(runtime(TILES)),
			storage,
		);
		expect(stale.status).toBe("pending");
		expect(stale.newSources).toEqual([TILES]);
	});

	test("minting refreshes an entry at most once a day and never a revoked one", () => {
		const day = MICRO_WIDGET_RUNTIME_REFRESH_MS;
		storage.items.set(
			microWidgetConsentKey(target),
			JSON.stringify({
				v: 2,
				policy: {},
				levels: {},
				runtime: [
					{ d: "imgSrc", s: TILES, l: "external", at: 10 },
					{ d: "imgSrc", s: OTHER_TILES, l: "external", at: 10 + day },
				],
				grantedAt: 1,
			}),
		);
		const at = () =>
			Object.fromEntries(
				(storedRecord().runtime as { s: string; at: number }[]).map((entry) => [
					entry.s,
					entry.at,
				]),
			);
		refreshMicroWidgetRuntimeConsent(
			target,
			[runtime(TILES), runtime(OTHER_TILES)],
			storage,
			20 + day,
		);
		expect(at()).toEqual({ [TILES]: 20 + day, [OTHER_TILES]: 10 + day });
		storage.items.set(microWidgetConsentRevokedKey(target), String(30 + day));
		refreshMicroWidgetRuntimeConsent(
			target,
			[runtime(TILES)],
			storage,
			40 + 3 * day,
		);
		expect(at()[TILES]).toBe(20 + day);
	});
});

describe("grants of what a dialog showed", () => {
	const wider = request({ ...network, downloads: true }, declared.levels);
	const both = request(wider.policy, wider.levels, [
		runtime(TILES),
		runtime(OTHER_TILES),
	]);
	const onlyNew = {
		...both,
		shown: { declared: false, runtime: [runtime(OTHER_TILES)] },
	};

	test("a project grant never persists a hidden declared part or runtime allowed for the session only", () => {
		grantMicroWidgetConsent(
			target,
			request(wider.policy, wider.levels, [runtime(TILES)]),
			"session",
			storage,
		);
		grantMicroWidgetConsent(target, onlyNew, "app", storage);
		expect(storedRecord().policy).toEqual({});
		expect(storedRecord().levels).toEqual({});
		expect(
			storedRecord().runtime.map((entry: { s: string }) => entry.s),
		).toEqual([OTHER_TILES]);
		expect(readMicroWidgetConsent(target, both, storage)).toBe("granted");

		resetMicroWidgetConsentForTests();
		const reloaded = evaluateMicroWidgetConsent(target, both, storage);
		expect(reloaded.declaredCovered).toBe(false);
		expect(reloaded.uncoveredRuntime.map((entry) => entry.source)).toEqual([
			TILES,
		]);
	});

	test("a hidden declared part leaves the project record's own declared part as it was", () => {
		grantMicroWidgetConsent(target, declared, "app", storage);
		const before = storedRecord();
		grantMicroWidgetConsent(target, wider, "session", storage);
		grantMicroWidgetConsent(target, onlyNew, "app", storage);
		const { runtime: added, ...kept } = storedRecord();
		expect(kept).toEqual(before);
		expect(added.map((entry: { s: string }) => entry.s)).toEqual([OTHER_TILES]);
		expect(evaluateMicroWidgetConsent(target, wider, storage).status).toBe(
			"granted",
		);
		resetMicroWidgetConsentForTests();
		expect(
			evaluateMicroWidgetConsent(target, wider, storage).newCapabilities,
		).toEqual(["downloads"]);
	});

	test("a hidden declared part keeps a capability-only legacy grant", () => {
		const legacy = "widget-capability-consent-app-app1-maps/chart";
		const owner = { ...target, packageId: "maps", widgetId: "chart" };
		storage.items.set(legacy, JSON.stringify(["workers"]));
		grantMicroWidgetConsent(
			owner,
			{
				policy: { workers: true },
				levels: {},
				runtime: [runtime(TILES)],
				shown: { declared: false, runtime: [runtime(TILES)] },
			},
			"app",
			storage,
		);
		expect(storage.items.has(legacy)).toBe(true);
		resetMicroWidgetConsentForTests();
		expect(readMicroWidgetConsent(owner, { workers: true }, storage)).toBe(
			"granted",
		);
	});
});

describe("runtime blocks and mute", () => {
	test("session runtime blocks feed extraction and are cleared by reopen, revoke and clear", () => {
		let notified = 0;
		const unsubscribe = subscribeMicroWidgetConsent(() => notified++);
		blockMicroWidgetRuntimeSources(target, [TILES]);
		blockMicroWidgetRuntimeSources(target, [TILES]);
		expect(notified).toBe(1);
		expect([...readMicroWidgetRuntimeBlocks(target)]).toEqual([TILES]);
		expect(
			readMicroWidgetRuntimeBlocks({ ...target, appId: "app2" }).size,
		).toBe(0);
		reopenMicroWidgetConsent(target);
		expect(readMicroWidgetRuntimeBlocks(target).size).toBe(0);
		blockMicroWidgetRuntimeSources(target, [TILES]);
		revokeMicroWidgetConsent(target, storage);
		expect(readMicroWidgetRuntimeBlocks(target).size).toBe(0);
		blockMicroWidgetRuntimeSources(target, [TILES]);
		clearMicroWidgetConsents({ appId: "app1" }, storage);
		expect(readMicroWidgetRuntimeBlocks(target).size).toBe(0);
		blockMicroWidgetRuntimeSources(target, [TILES]);
		clearPackageMicroWidgetConsents(target.packageId, storage);
		expect(readMicroWidgetRuntimeBlocks(target).size).toBe(0);
		unsubscribe();
	});

	test("a runtime block never forgets the declared grant", () => {
		grantMicroWidgetConsent(target, declared, "app", storage);
		blockMicroWidgetRuntimeSources(target, [TILES]);
		expect(readMicroWidgetConsent(target, declared, storage)).toBe("granted");
		expect(storedRecord()).not.toBeNull();
	});

	test("stop asking is stored for the project and survives a declared re-grant", () => {
		grantMicroWidgetConsent(target, declared, "app", storage);
		muteMicroWidgetRuntime(target, storage);
		expect(storedRecord().runtimeMuted).toBe(true);
		resetMicroWidgetConsentForTests();
		expect(evaluateMicroWidgetConsent(target, declared, storage).muted).toBe(
			true,
		);
		grantMicroWidgetConsent(target, declared, "app", storage);
		expect(storedRecord().runtimeMuted).toBe(true);
		expect(
			listMicroWidgetConsents("app1", storage).map(
				(entry) => entry.runtimeMuted,
			),
		).toEqual([true]);
		unmuteMicroWidgetRuntime(target, storage);
		expect(evaluateMicroWidgetConsent(target, declared, storage).muted).toBe(
			false,
		);
		expect(readMicroWidgetConsent(target, declared, storage)).toBe("granted");
	});

	test("without a project, stop asking lasts for the session only", () => {
		const detached = { ...target, appId: undefined };
		muteMicroWidgetRuntime(detached, storage);
		expect(storage.items.size).toBe(0);
		expect(evaluateMicroWidgetConsent(detached, {}, storage).muted).toBe(true);
		unmuteMicroWidgetRuntime(detached, storage);
		expect(evaluateMicroWidgetConsent(detached, {}, storage).muted).toBe(false);
		expect(listMicroWidgetConsents(undefined, storage)).toEqual([]);
	});

	test("revoke clears the mute with everything else", () => {
		muteMicroWidgetRuntime(target, storage);
		revokeMicroWidgetConsent(target, storage);
		expect(evaluateMicroWidgetConsent(target, {}, storage).muted).toBe(false);
		expect(storage.items.has(microWidgetConsentKey(target))).toBe(false);
	});

	test("the manage list shows live runtime approvals", () => {
		grantMicroWidgetConsent(
			target,
			request(declared.policy, declared.levels, [runtime(TILES)]),
			"app",
			storage,
		);
		const [entry, ...rest] = listMicroWidgetConsents("app1", storage);
		expect(rest).toEqual([]);
		expect(entry.levels).toEqual(declared.levels);
		expect(entry.runtime.map(({ d, s, l }) => ({ d, s, l }))).toEqual([
			{ d: "imgSrc", s: TILES, l: "external" },
		]);
		grantMicroWidgetConsent(
			target,
			request(declared.policy, declared.levels, [runtime(OTHER_TILES)]),
			"session",
			storage,
		);
		expect(
			listMicroWidgetConsents("app1", storage).map((item) => item.scope),
		).toEqual(["app", "session"]);
	});
});
