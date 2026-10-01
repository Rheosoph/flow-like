import {
	afterAll,
	beforeAll,
	beforeEach,
	expect,
	setSystemTime,
	test,
} from "bun:test";
import {
	DeviceLockHeldError,
	DeviceLockUnsupportedError,
	type LocalDeviceVault,
	acquireDeviceLock,
	addDeviceVault,
	completeAccountRecovery,
	deleteDeviceVault,
	deviceIdentityKey,
	forgetDeviceIdentityPin,
	listAccountRecoveryStates,
	listDeviceVaults,
	markPasswordChangedSinceBackup,
	pinDeviceIdentity,
	pinnedDeviceIdentity,
	queryDeviceLock,
	readAccountRecoveryState,
	readDeviceIdentityPins,
	readInventoryAnchors,
	readStoragePersistence,
	requestPersistentDeviceStorage,
	stageAccountRecovery,
	updateFleetState,
	updateInventoryAnchors,
} from "./storage";
import type { AccountRecoveryWrite, DeviceReceipt } from "./types";

const scope = {
	issuer: "issuer",
	account: "owner",
	apiOrigin: "https://hub.test",
	profileId: "profile",
};
const otherScope = { ...scope, profileId: "other-profile" };

type Range = { lower: string; upper: string };
const stores = new Map<string, Map<string, unknown>>();
let failCommit = false;

function inRange(key: string, range: Range | undefined) {
	return !range || (key >= range.lower && key <= range.upper);
}
function request<T>(result: T) {
	const value = {
		result,
		error: null as { name: string } | null,
		onsuccess: undefined as (() => void) | undefined,
		onerror: undefined as
			| ((event: { preventDefault(): void }) => void)
			| undefined,
	};
	queueMicrotask(() => value.onsuccess?.());
	return value;
}

// In-memory IndexedDB: staged per transaction, committed after the last request settles.
function fakeIndexedDb() {
	return {
		open() {
			const opened = request({
				close() {},
				transaction(names: string | string[]) {
					const selected = typeof names === "string" ? [names] : names;
					const staged = new Map(
						selected.map((name) => [name, new Map(stores.get(name) ?? [])]),
					);
					let aborted = false;
					let pending = 0;
					const settle = () =>
						queueMicrotask(() => {
							if (--pending > 0 || aborted) return;
							if (failCommit) {
								failCommit = false;
								tx.abort();
								return;
							}
							for (const [name, rows] of staged) stores.set(name, rows);
							tx.oncomplete?.();
						});
					const track = <T>(result: T) => {
						pending++;
						const value = request(result);
						queueMicrotask(() => queueMicrotask(settle));
						return value;
					};
					const tx = {
						oncomplete: undefined as (() => void) | undefined,
						onerror: undefined as (() => void) | undefined,
						onabort: undefined as (() => void) | undefined,
						abort() {
							aborted = true;
							queueMicrotask(() => tx.onabort?.());
						},
						objectStore(name: string) {
							const rows = staged.get(name);
							if (!rows)
								throw new Error(`Store ${name} is outside this transaction`);
							const sorted = (range?: Range) =>
								[...rows.keys()].filter((key) => inRange(key, range)).sort();
							return {
								get: (key: string) => track(structuredClone(rows.get(key))),
								getAll: (range?: Range) =>
									track(
										sorted(range).map((key) => structuredClone(rows.get(key))),
									),
								getAllKeys: (range?: Range) => track(sorted(range)),
								put(value: unknown, key: string) {
									rows.set(key, structuredClone(value));
									return track(undefined);
								},
								add(value: unknown, key: string) {
									const result = track(undefined);
									if (rows.has(key)) {
										result.error = { name: "ConstraintError" };
										queueMicrotask(() => {
											result.onerror?.({ preventDefault() {} });
											tx.abort();
										});
									} else rows.set(key, structuredClone(value));
									return result;
								},
								delete(key: string | Range) {
									for (const row of [...rows.keys()])
										if (
											typeof key === "string" ? row === key : inRange(row, key)
										)
											rows.delete(row);
									return track(undefined);
								},
							};
						},
					};
					pending++;
					queueMicrotask(settle);
					return tx;
				},
			});
			return opened;
		},
	};
}

type Held = { reject: (error: unknown) => void };
function fakeLocks() {
	const held = new Map<string, Held>();
	return {
		held,
		request(
			name: string,
			options: { ifAvailable?: boolean; steal?: boolean },
			run: (lock: object | null) => Promise<void>,
		) {
			return new Promise<void>((resolve, reject) => {
				const current = held.get(name);
				if (current && !options.steal) {
					run(null).then(resolve, reject);
					return;
				}
				if (current) {
					held.delete(name);
					current.reject(new DOMException("Lock stolen", "AbortError"));
				}
				const entry: Held = { reject };
				held.set(name, entry);
				run({ name }).then(() => {
					if (held.get(name) === entry) held.delete(name);
					resolve();
				}, reject);
			});
		},
		async query() {
			return { held: [...held.keys()].map((name) => ({ name })) };
		},
	};
}

const original = {
	indexedDB: Object.getOwnPropertyDescriptor(globalThis, "indexedDB"),
	IDBKeyRange: Object.getOwnPropertyDescriptor(globalThis, "IDBKeyRange"),
	navigator: Object.getOwnPropertyDescriptor(globalThis, "navigator"),
};
let locks = fakeLocks();
let storage: {
	persisted?: () => Promise<boolean>;
	persist?: () => Promise<boolean>;
} = {};
function setNavigator(value: object) {
	Object.defineProperty(globalThis, "navigator", { configurable: true, value });
}
beforeAll(() => {
	Object.defineProperty(globalThis, "indexedDB", {
		configurable: true,
		value: fakeIndexedDb(),
	});
	Object.defineProperty(globalThis, "IDBKeyRange", {
		configurable: true,
		value: { bound: (lower: string, upper: string) => ({ lower, upper }) },
	});
});
afterAll(() => {
	for (const [name, descriptor] of Object.entries(original))
		if (descriptor) Object.defineProperty(globalThis, name, descriptor);
		else Reflect.deleteProperty(globalThis, name);
	setSystemTime();
});
beforeEach(() => {
	stores.clear();
	failCommit = false;
	locks = fakeLocks();
	storage = {};
	setNavigator({ locks, storage });
});

function vault(deviceId: string): LocalDeviceVault {
	const key = { kty: "OKP" as const, crv: "Ed25519" as const, x: "controller" };
	return {
		deviceId,
		grantId: "owner",
		manifestJws: "signed-manifest",
		controllerPublic: {
			device_id: deviceId,
			endpoint_id: "endpoint",
			controller_key: key,
			archive_key: Array(32).fill(7),
			telemetry_member: { endpoint_id: "endpoint", signing_key: key },
		},
		controllerVault: new Uint8Array(80).fill(1),
	};
}
function receipt(
	deviceId: string,
	enrollmentId: string,
	auth: string,
): DeviceReceipt {
	const key = (x: string) => ({
		kty: "OKP" as const,
		crv: "Ed25519" as const,
		x,
	});
	return {
		enrollment_id: enrollmentId,
		device_id: deviceId,
		owner_id: scope.account,
		name: deviceId,
		identity: {
			auth_key: key(auth),
			telemetry_key: key("telemetry"),
			management_key: Array(32).fill(3),
		},
		manifest_jws: "manifest",
		binding_jws: "binding",
		registered_at: 1,
		auth_epoch: 1,
	};
}
function backup(revision: number): AccountRecoveryWrite {
	return {
		public_key: { kty: "OKP", crv: "Ed25519", x: "controller" },
		ciphertext: `ciphertext-${revision}`,
		revision,
		proof_jws: `proof-${revision}`,
	};
}

test("lists this scope's vaults and reads identity pins newest first", async () => {
	await addDeviceVault(scope, vault("dev"));
	await addDeviceVault(scope, vault("dev-2"));
	await addDeviceVault(otherScope, vault("elsewhere"));
	setSystemTime(new Date(1_000));
	await pinDeviceIdentity(scope, "dev", receipt("dev", "first", "auth-a"));
	setSystemTime(new Date(5_000));
	await pinDeviceIdentity(scope, "dev", receipt("dev", "second", "auth-b"));
	setSystemTime();

	expect((await listDeviceVaults(scope)).map((row) => row.deviceId)).toEqual([
		"dev",
		"dev-2",
	]);
	const pins = await readDeviceIdentityPins(scope, "dev");
	expect(pins.map((pin) => [pin.enrollmentId, pin.pinnedAt])).toEqual([
		["second", 5_000],
		["first", 1_000],
	]);
	expect(pinnedDeviceIdentity(pins[0])).toEqual(
		receipt("dev", "second", "auth-b").identity,
	);
	expect(pins[0].identity).toBe(
		deviceIdentityKey(receipt("dev", "second", "auth-b").identity),
	);
	expect(pinnedDeviceIdentity({ identity: "not json" })).toBeUndefined();
	expect(await readDeviceIdentityPins(scope, "dev-2")).toEqual([]);

	await forgetDeviceIdentityPin(scope, "dev", "first");
	expect(
		(await readDeviceIdentityPins(scope, "dev")).map((pin) => pin.enrollmentId),
	).toEqual(["second"]);
	await pinDeviceIdentity(scope, "dev", receipt("dev", "first", "auth-c"));
	await forgetDeviceIdentityPin(scope, "dev");
	expect(await readDeviceIdentityPins(scope, "dev")).toEqual([]);
	expect(await listDeviceVaults(scope)).toHaveLength(2);
});

test("deleting a device's keys removes every local row of that device only", async () => {
	for (const [account, deviceId] of [
		[scope, "dev"],
		[scope, "dev-2"],
		[otherScope, "dev"],
	] as const) {
		await addDeviceVault(account, vault(deviceId));
		await pinDeviceIdentity(account, deviceId, receipt(deviceId, "e1", "auth"));
		await markPasswordChangedSinceBackup(account, deviceId, true);
		await updateFleetState(account, deviceId, "controller", (current) => ({
			...current,
			revision: 2,
		}));
		await updateInventoryAnchors(account, deviceId, "controller", () => ({
			device: { scope: { kind: "device" }, revision: 1, digest: "d" },
		}));
	}
	const snapshots = stores.get("snapshots") ?? new Map();
	snapshots.set(JSON.stringify([accountKey(scope), "dev", "ep", "aud"]), {});
	snapshots.set(JSON.stringify([accountKey(scope), "dev-2", "ep", "aud"]), {});
	stores.set("snapshots", snapshots);
	const before = rowCount();

	await deleteDeviceVault(scope, "dev");

	expect(rowCount()).toBe(before - 6);
	for (const keys of stores.values())
		for (const key of keys.keys()) {
			const [account, deviceId] = JSON.parse(key) as string[];
			expect(account === accountKey(scope) && deviceId === "dev").toBe(false);
		}
	expect((await listDeviceVaults(scope)).map((row) => row.deviceId)).toEqual([
		"dev-2",
	]);
	expect(await listDeviceVaults(otherScope)).toHaveLength(1);
	expect(await readDeviceIdentityPins(otherScope, "dev")).toHaveLength(1);
});

function accountKey(account: typeof scope) {
	return JSON.stringify([
		account.issuer,
		account.account,
		account.apiOrigin,
		account.profileId,
	]);
}
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
function rowCount() {
	return [...stores.values()].reduce((sum, rows) => sum + rows.size, 0);
}

test("inventory anchors compare-and-set in one transaction and abort on conflict", async () => {
	const seen: boolean[] = [];
	const first = await updateInventoryAnchors(
		scope,
		"dev",
		"controller",
		(current, stored) => {
			seen.push(stored);
			expect(current).toEqual({});
			return {
				device: { scope: { kind: "device" }, revision: 1, digest: "a" },
			};
		},
	);
	await updateInventoryAnchors(
		scope,
		"dev",
		"controller",
		(current, stored) => {
			seen.push(stored);
			return {
				...current,
				device: { ...current.device, revision: 2, digest: "b" },
			};
		},
	);
	expect(seen).toEqual([false, true]);
	expect(first.device.revision).toBe(1);
	await expect(
		updateInventoryAnchors(scope, "dev", "controller", () => {
			throw new Error("Saved inventory moved backwards.");
		}),
	).rejects.toThrow("moved backwards");
	await expect(
		updateInventoryAnchors(scope, "dev", "controller", () => ({
			huge: {
				scope: { kind: "device" },
				revision: 3,
				digest: "x".repeat(140_000),
			},
		})),
	).rejects.toThrow("local limit");
	failCommit = true;
	await expect(
		updateInventoryAnchors(scope, "dev", "controller", () => ({})),
	).rejects.toThrow("not committed");
	expect(await readInventoryAnchors(scope, "dev", "controller")).toEqual({
		device: { scope: { kind: "device" }, revision: 2, digest: "b" },
	});
	expect(await readInventoryAnchors(scope, "dev", "other-key")).toEqual({});
	expect(await readInventoryAnchors(otherScope, "dev", "controller")).toEqual(
		{},
	);
	expect(
		(await updateFleetState(scope, "dev", "controller", (current) => current))
			.revision,
	).toBe(0);
});

test("a password change flags the account backup until a backup sealed after it is published", async () => {
	await markPasswordChangedSinceBackup(scope, "dev", true);
	await markPasswordChangedSinceBackup(otherScope, "elsewhere", true);
	expect(await listAccountRecoveryStates(scope)).toEqual({
		dev: { revision: 0, passwordChangedSinceBackup: true },
	});

	await stageAccountRecovery(scope, "dev", "digest-1", backup(1));
	await completeAccountRecovery(scope, "dev", backup(1));
	expect(await readAccountRecoveryState(scope, "dev")).toEqual({
		revision: 1,
		sourceDigest: "digest-1",
	});

	await stageAccountRecovery(scope, "dev", "digest-2", backup(2));
	await markPasswordChangedSinceBackup(scope, "dev", true);
	await completeAccountRecovery(scope, "dev", backup(2));
	expect(await readAccountRecoveryState(scope, "dev")).toEqual({
		revision: 2,
		sourceDigest: "digest-2",
		passwordChangedSinceBackup: true,
	});
	await markPasswordChangedSinceBackup(scope, "dev", false);
	expect(
		(await readAccountRecoveryState(scope, "dev")).passwordChangedSinceBackup,
	).toBeUndefined();
	expect(Object.keys(await listAccountRecoveryStates(otherScope))).toEqual([
		"elsewhere",
	]);
});

test("reading storage persistence never asks the browser to persist", async () => {
	let asked = 0;
	storage.persist = async () => {
		asked++;
		return true;
	};
	storage.persisted = async () => false;
	expect(await readStoragePersistence()).toBe("denied");
	storage.persisted = async () => true;
	expect(await readStoragePersistence()).toBe("persisted");
	storage.persisted = async () => {
		throw new Error("blocked");
	};
	expect(await readStoragePersistence()).toBe("unavailable");
	expect(asked).toBe(0);
	storage.persisted = async () => false;
	expect(await requestPersistentDeviceStorage()).toBe("persisted");
	expect(asked).toBe(1);
	setNavigator({ locks });
	expect(await readStoragePersistence()).toBe("unavailable");
});

test("the device lock reports its holder, steals on request and tells the loser", async () => {
	expect(await queryDeviceLock(scope, "dev")).toBe("free");
	const release = await acquireDeviceLock(scope, "dev");
	expect(await queryDeviceLock(scope, "dev")).toBe("held_here");
	expect(await queryDeviceLock(otherScope, "dev")).toBe("free");
	release();
	await flush();
	expect(await queryDeviceLock(scope, "dev")).toBe("free");

	const name = JSON.stringify([accountKey(scope), "dev", "unlock"]);
	let otherWindowLost: unknown;
	void locks
		.request(name, {}, () => new Promise<void>(() => {}))
		.catch((error) => {
			otherWindowLost = error;
		});
	expect(await queryDeviceLock(scope, "dev")).toBe("held_elsewhere");
	await expect(acquireDeviceLock(scope, "dev")).rejects.toBeInstanceOf(
		DeviceLockHeldError,
	);

	let lost = 0;
	const stolen = await acquireDeviceLock(scope, "dev", {
		steal: true,
		onLost: () => lost++,
	});
	await flush();
	expect((otherWindowLost as DOMException).name).toBe("AbortError");
	expect(await queryDeviceLock(scope, "dev")).toBe("held_here");

	const winner = await acquireDeviceLock(scope, "dev", { steal: true });
	await flush();
	expect(lost).toBe(1);
	expect(await queryDeviceLock(scope, "dev")).toBe("held_here");
	stolen();
	winner();
	await flush();
	expect(lost).toBe(1);
	expect(await queryDeviceLock(scope, "dev")).toBe("free");

	setNavigator({ storage });
	expect(await queryDeviceLock(scope, "dev")).toBe("unsupported");
	await expect(acquireDeviceLock(scope, "dev")).rejects.toBeInstanceOf(
		DeviceLockUnsupportedError,
	);
});
