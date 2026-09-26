import { afterEach, beforeEach, expect, test } from "bun:test";
import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import {
	checkInventoryAnchor,
	createInventoryWriter,
	inventoryObservation,
	inventoryScopeContains,
	openInventoryView,
	visibleInventory,
} from "./inventory";
import { accountStorageKey } from "./storage";
import type {
	BrowserController,
	EncryptedInventory,
	InventoryBinding,
	InventoryView,
	PlacementStatus,
} from "./types";

const storageDescriptor = Object.getOwnPropertyDescriptor(
	globalThis,
	"localStorage",
);
beforeEach(() => {
	const values = new Map<string, string>();
	Object.defineProperty(globalThis, "localStorage", {
		configurable: true,
		value: {
			getItem: (key: string) => values.get(key) ?? null,
			setItem: (key: string, value: string) => values.set(key, value),
		},
	});
});
afterEach(() => {
	if (storageDescriptor)
		Object.defineProperty(globalThis, "localStorage", storageDescriptor);
	else Reflect.deleteProperty(globalThis, "localStorage");
});

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
				{
					issuer: "issuer",
					account: "account",
					apiOrigin: "https://api.example",
					profileId: "profile",
				},
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

test("reading inventory cannot lower an anchor advanced by a queued write during decryption", async () => {
	const account = {
		issuer: "issuer",
		account: "account",
		apiOrigin: "https://api.example",
		profileId: "profile",
	};
	const key = `flow-like/inventory/${JSON.stringify([accountStorageKey(account), "device", "key"])}`;
	const floor = {
		scope: { kind: "project" as const, project_id: "project" },
		revision: 2,
		digest: "newer-ciphertext",
	};
	const encrypted: EncryptedInventory = {
		binding: {
			issuer: account.issuer,
			api_origin: account.apiOrigin,
			account_id: account.account,
			device_id: "device",
			controller_key: { kty: "OKP", crv: "Ed25519", x: "key" },
			scope: floor.scope,
			revision: 1,
		},
		ciphertext: "old-ciphertext",
	};
	let concurrent: Record<string, typeof floor> = { "project/project": floor };
	const controller = {
		publicBundle: () => ({
			device_id: "device",
			controller_key: encrypted.binding.controller_key,
		}),
		openInventory: () => {
			localStorage.setItem(key, JSON.stringify(concurrent));
			return new TextEncoder().encode(JSON.stringify(observation(10)));
		},
	} as unknown as BrowserController;
	const view = { scopes: [floor.scope], observations: [encrypted] };
	await expect(openInventoryView(account, controller, view)).rejects.toThrow(
		"backwards",
	);
	expect(
		JSON.parse(localStorage.getItem(key) ?? "{}")["project/project"],
	).toEqual(floor);
	// An unrelated, currently unreadable scope still keeps its newer floor.
	concurrent = {
		"project/other": {
			...floor,
			scope: { kind: "project", project_id: "other" },
		},
	};
	localStorage.setItem(key, "{}");
	await openInventoryView(account, controller, view);
	const anchors = JSON.parse(localStorage.getItem(key) ?? "{}");
	expect(anchors["project/other"]).toEqual(concurrent["project/other"]);
	expect(anchors["project/project"].revision).toBe(1);
});
