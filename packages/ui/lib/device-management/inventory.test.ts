import { afterEach, beforeEach, expect, test } from "bun:test";
import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import {
	SnapshotIntegrityError,
	checkInventoryAnchor,
	createInventoryWriter,
	inventoryObservation,
	inventoryScopeContains,
	legacyInventoryAnchorKey,
	openInventoryView,
	readSavedInventory,
	visibleInventory,
} from "./inventory";
import {
	type InventoryAnchors,
	readInventoryAnchors,
	updateInventoryAnchors,
} from "./storage";
import type {
	BrowserController,
	EncryptedInventory,
	InventoryBinding,
	InventoryScope,
	InventoryView,
	PlacementStatus,
} from "./types";

const originals = {
	indexedDB: Object.getOwnPropertyDescriptor(globalThis, "indexedDB"),
	localStorage: Object.getOwnPropertyDescriptor(globalThis, "localStorage"),
};
let rows = new Map<string, unknown>();
let legacy = new Map<string, string>();
let tail: Promise<void> = Promise.resolve();

/** Transactions on the one store run one after another and commit atomically, as in IndexedDB. */
function transaction() {
	let release!: () => void;
	const turn = tail;
	tail = new Promise((resolve) => {
		release = resolve;
	});
	let staged = rows;
	let aborted = false;
	const tx = {
		oncomplete: undefined as (() => void) | undefined,
		onabort: undefined as (() => void) | undefined,
		abort() {
			aborted = true;
			queueMicrotask(() => {
				tx.onabort?.();
				release();
			});
		},
		objectStore: () => ({
			get(key: string) {
				const request = {
					result: undefined as unknown,
					onsuccess: undefined as (() => void) | undefined,
				};
				void turn.then(() => {
					staged = new Map(rows);
					request.result = structuredClone(staged.get(key));
					request.onsuccess?.();
					queueMicrotask(() => {
						if (aborted) return;
						rows = staged;
						tx.oncomplete?.();
						release();
					});
				});
				return request;
			},
			put(value: unknown, key: string) {
				staged.set(key, structuredClone(value));
			},
		}),
	};
	return tx;
}
const fakeIndexedDb = {
	open() {
		const request = {
			result: { close() {}, transaction },
			onsuccess: undefined as (() => void) | undefined,
		};
		queueMicrotask(() => request.onsuccess?.());
		return request;
	},
};
function setLocalStorage(blocked: boolean) {
	Object.defineProperty(globalThis, "localStorage", {
		configurable: true,
		get() {
			if (blocked) throw new Error("The operation is insecure.");
			return {
				getItem: (key: string) => legacy.get(key) ?? null,
				setItem: (key: string, value: string) => legacy.set(key, value),
				removeItem: (key: string) => legacy.delete(key),
			};
		},
	});
}
beforeEach(() => {
	rows = new Map();
	legacy = new Map();
	tail = Promise.resolve();
	Object.defineProperty(globalThis, "indexedDB", {
		configurable: true,
		value: fakeIndexedDb,
	});
	setLocalStorage(false);
});
afterEach(() => {
	for (const [name, descriptor] of Object.entries(originals))
		if (descriptor) Object.defineProperty(globalThis, name, descriptor);
		else Reflect.deleteProperty(globalThis, name);
});

const account = {
	issuer: "issuer",
	account: "account",
	apiOrigin: "https://api.example",
	profileId: "profile",
};
const anchors = () => readInventoryAnchors(account, "device", "key");

const row: PlacementStatus = {
	id: "placement",
	project_id: "project",
	deployment_id: "deployment",
	revision: "rev",
	desired_state: "running",
	observed_state: "running",
	config_revision: 1,
	intent_revision: 1,
	applied_revision: 1,
	desired_replicas: 1,
	running_replicas: 1,
	ready_replicas: 1,
	max_replicas: 1,
	replicas: [{ slot: 0, observed_state: "running", applied_revision: 1 }],
};
test("retained inventory excludes unrecognized config, tokens and secret values", () => {
	const result = inventoryObservation(
		{
			device_id: "device",
			boot_id: "boot",
			observed_at: 10,
			placements: [
				{
					...row,
					secret: "password",
					config: { token: "private" },
				} as PlacementStatus,
			],
		},
		{ kind: "device" },
	);
	expect(JSON.stringify(result)).not.toContain("password");
	expect(JSON.stringify(result)).not.toContain("private");
	expect(result.placements[0]).toEqual(row);
});
test("narrow Status scope does not reveal a broader retained snapshot", () => {
	expect(
		inventoryScopeContains(
			{ kind: "placement", project_id: "project", placement_id: "placement" },
			{ kind: "project", project_id: "project" },
		),
	).toBe(false);
	expect(
		inventoryScopeContains(
			{ kind: "project", project_id: "other" },
			{ kind: "placement", project_id: "project", placement_id: "placement" },
		),
	).toBe(false);
});
test("revision floors reject cloud rollback and equal revision substitution", () => {
	const encrypted = { binding: { revision: 4 } } as EncryptedInventory;
	const previous = {
		scope: { kind: "device" as const },
		revision: 5,
		digest: "digest",
	};
	expect(() => checkInventoryAnchor(previous, encrypted, "digest")).toThrow(
		"backwards",
	);
	encrypted.binding.revision = 5;
	expect(() => checkInventoryAnchor(previous, encrypted, "other")).toThrow(
		"backwards",
	);
	expect(() =>
		checkInventoryAnchor(previous, encrypted, "digest"),
	).not.toThrow();
});
test("a newer empty project inspection removes stale placements from broader history", () => {
	const old = {
		scope: { kind: "device" as const },
		device_id: "device",
		boot_id: "old",
		observed_at: 10,
		placements: [row],
	};
	const next = {
		scope: { kind: "project" as const, project_id: "project" },
		device_id: "device",
		boot_id: "new",
		observed_at: 20,
		placements: [],
	};
	expect(visibleInventory([next, old])).toEqual([]);
	expect(
		visibleInventory([
			{ ...next, scope: { kind: "project", project_id: "other" } },
			old,
		]),
	).toEqual([{ row, at: 10 }]);
});

function writerFixture() {
	let active = true;
	const sealed: EncryptedInventory[] = [];
	const view: InventoryView = {
		scopes: [{ kind: "device" }],
		observations: [],
	};
	const controller = {
		publicBundle: () => {
			if (!active) throw new Error("controller freed");
			return {
				device_id: "device",
				controller_key: { kty: "OKP", crv: "Ed25519", x: "key" },
			};
		},
		sealInventory: (binding: InventoryBinding, bytes: Uint8Array) => {
			if (!active) throw new Error("controller freed");
			const value = {
				binding,
				ciphertext: btoa(new TextDecoder().decode(bytes)),
			};
			sealed.push(value);
			return value;
		},
	} as unknown as BrowserController;
	const writes: EncryptedInventory[] = [];
	const api = {
		get: async () => view,
		put: async (
			_profile: IProfile,
			_url: string,
			encrypted: EncryptedInventory,
		) => {
			writes.push(encrypted);
			view.observations = [encrypted];
			return view;
		},
	};
	return {
		api,
		sealed,
		writes,
		close: () => {
			active = false;
		},
		writer: () =>
			createInventoryWriter(
				api as unknown as IApiState,
				{} as IProfile,
				account,
				controller,
				"owner",
				() => active,
			),
	};
}

const observation = (observed_at: number) => ({
	device_id: "device",
	boot_id: "boot",
	observed_at,
	placements: [row],
});

test("accepted inspections are sealed before close and ciphertext commits in revision order", async () => {
	const fixture = writerFixture();
	const write = await fixture.writer();
	const first = write(observation(10));
	const second = write(observation(11));
	expect(fixture.sealed.map((row) => row.binding.revision)).toEqual([1, 2]);
	expect(fixture.writes).toHaveLength(0);
	fixture.close();
	await Promise.all([first, second]);
	expect(fixture.writes.map((row) => row.binding.revision)).toEqual([1, 2]);
	expect(() => write(observation(12))).toThrow("cancelled");
});

test("inventory CAS failure stops queued revisions and is reported to every waiting caller", async () => {
	const fixture = writerFixture();
	fixture.api.put = async () => {
		throw new Error("conflicting revision");
	};
	const write = await fixture.writer();
	const results = await Promise.allSettled([
		write(observation(10)),
		write(observation(11)),
	]);
	expect(results.every((result) => result.status === "rejected")).toBe(true);
	expect(fixture.writes).toHaveLength(0);
	expect(() => write(observation(12))).toThrow("Reconnect");
});

test("closing during inventory preparation never accesses a freed controller", async () => {
	const fixture = writerFixture();
	fixture.api.get = async () => {
		fixture.close();
		return { scopes: [{ kind: "device" }], observations: [] };
	};
	await expect(fixture.writer()).rejects.toThrow("cancelled");
	expect(fixture.sealed).toHaveLength(0);
});

const projectScope: InventoryScope = { kind: "project", project_id: "project" };
function sealed(revision: number, scope = projectScope): EncryptedInventory {
	return {
		binding: {
			issuer: account.issuer,
			api_origin: account.apiOrigin,
			account_id: account.account,
			device_id: "device",
			controller_key: { kty: "OKP", crv: "Ed25519", x: "key" },
			scope,
			revision,
		},
		ciphertext: `ciphertext-${revision}`,
	};
}
function reader(onOpen: () => void = () => undefined) {
	return {
		publicBundle: () => ({
			device_id: "device",
			controller_key: { kty: "OKP", crv: "Ed25519", x: "key" },
		}),
		openInventory: () => {
			onOpen();
			return new TextEncoder().encode(JSON.stringify(observation(10)));
		},
	} as unknown as BrowserController;
}
const viewOf = (...observations: EncryptedInventory[]): InventoryView => ({
	scopes: [projectScope],
	observations,
});

test("reading inventory cannot lower an anchor advanced by a queued write during decryption", async () => {
	const floor = {
		scope: projectScope,
		revision: 2,
		digest: "newer-ciphertext",
	};
	let concurrent: InventoryAnchors = { "project/project": floor };
	let write: Promise<unknown> = Promise.resolve();
	const controller = reader(() => {
		write = updateInventoryAnchors(account, "device", "key", (current) => ({
			...current,
			...concurrent,
		}));
	});
	await expect(
		openInventoryView(account, controller, viewOf(sealed(1))),
	).rejects.toThrow("backwards");
	await write;
	expect((await anchors())["project/project"]).toEqual(floor);
	// An unrelated, currently unreadable scope still keeps its newer floor.
	const other = { ...floor, scope: { kind: "project", project_id: "other" } };
	concurrent = { "project/other": other as InventoryAnchors[string] };
	rows = new Map();
	await openInventoryView(account, controller, viewOf(sealed(1)));
	await write;
	const stored = await anchors();
	expect(stored["project/other"]).toEqual(concurrent["project/other"]);
	expect(stored["project/project"].revision).toBe(1);
});

test("rollback and access failures are integrity errors; unavailable storage is not", async () => {
	await openInventoryView(account, reader(), viewOf(sealed(2)));
	const rollback = await openInventoryView(
		account,
		reader(),
		viewOf(sealed(1)),
	).catch((error: unknown) => error);
	expect(rollback).toBeInstanceOf(SnapshotIntegrityError);
	const outside = await openInventoryView(account, reader(), {
		scopes: [projectScope],
		observations: [sealed(3, { kind: "device" })],
	}).catch((error: unknown) => error);
	expect(outside).toBeInstanceOf(SnapshotIntegrityError);
	Reflect.deleteProperty(globalThis, "indexedDB");
	const storage = await openInventoryView(
		account,
		reader(),
		viewOf(sealed(3)),
	).catch((error: unknown) => error);
	expect((storage as Error).message).toContain("unavailable");
	expect(storage).not.toBeInstanceOf(SnapshotIntegrityError);
});

test("old localStorage floors are enforced, imported once into IndexedDB and then removed", async () => {
	const key = legacyInventoryAnchorKey(account, "device", "key");
	const legacyFloor = { scope: projectScope, revision: 2, digest: "legacy" };
	legacy.set(
		key,
		JSON.stringify({
			"project/project": legacyFloor,
			"project/broken": { revision: "two" },
		}),
	);
	await expect(
		openInventoryView(account, reader(), viewOf(sealed(1))),
	).rejects.toThrow("backwards");
	expect(legacy.has(key)).toBe(true);
	await openInventoryView(account, reader(), viewOf(sealed(3)));
	expect(legacy.has(key)).toBe(false);
	const imported = await anchors();
	expect(Object.keys(imported)).toEqual(["project/project"]);
	expect(imported["project/project"].revision).toBe(3);
	legacy.set(
		key,
		JSON.stringify({ "project/project": { ...legacyFloor, revision: 9 } }),
	);
	await expect(
		openInventoryView(account, reader(), viewOf(sealed(3))),
	).resolves.toHaveLength(1);
	expect((await anchors())["project/project"].revision).toBe(3);
});

test("blocked localStorage never fails a read and the floor still lands in IndexedDB", async () => {
	setLocalStorage(true);
	await expect(
		openInventoryView(account, reader(), viewOf(sealed(1))),
	).resolves.toHaveLength(1);
	expect((await anchors())["project/project"].revision).toBe(1);
	await expect(
		openInventoryView(account, reader(), viewOf(sealed(0))),
	).rejects.toBeInstanceOf(SnapshotIntegrityError);
});

test("the saved inventory is read for the vault's grant and opened with its floors", async () => {
	const gets: string[] = [];
	const api = {
		get: async (_profile: IProfile, url: string) => {
			gets.push(url);
			return viewOf(sealed(1));
		},
	} as unknown as IApiState;
	const saved = await readSavedInventory(
		api,
		{} as IProfile,
		account,
		reader(),
		"grant one",
	);
	expect(gets).toEqual(["devices/device/inventory/key?grant_id=grant%20one"]);
	expect(saved).toEqual([
		{ ...observation(10), scope: projectScope, placements: [row] },
	]);
	expect((await anchors())["project/project"].revision).toBe(1);
});
