import { afterAll, beforeEach, expect, test } from "bun:test";
import { changeDevicePassword } from "./password";
import {
	type LocalDeviceVault,
	addDeviceVault,
	controllerBackup,
	encryptedControllerBackup,
	readDeviceVault,
	replaceRewrappedVault,
} from "./storage";
import type { DeviceCrypto } from "./types";

const scope = {
	issuer: "issuer",
	account: "reader",
	apiOrigin: "https://hub.test",
	profileId: "profile",
};
const stores = new Map<string, Map<string, unknown>>();
let failCommit = false;
let lockHeld = false;
const originalIdb = Object.getOwnPropertyDescriptor(globalThis, "indexedDB");
const originalNavigator = Object.getOwnPropertyDescriptor(
	globalThis,
	"navigator",
);

// A transaction harness injects a failed commit after put, before persistent state changes.
Object.defineProperty(globalThis, "indexedDB", {
	configurable: true,
	value: {
		open() {
			const request: { onsuccess?: () => void; result: unknown } = {
				result: {
					close() {},
					transaction(name: string) {
						const persisted = stores.get(name) ?? new Map();
						stores.set(name, persisted);
						const staged = new Map(persisted);
						let aborted = false;
						let scheduled = false;
						const tx = {
							oncomplete: undefined as (() => void) | undefined,
							onabort: undefined as (() => void) | undefined,
							abort() {
								aborted = true;
								queueMicrotask(() => tx.onabort?.());
							},
							objectStore() {
								return {
									get(key: string) {
										const result = {
											result: structuredClone(staged.get(key)),
											onsuccess: undefined as (() => void) | undefined,
										};
										queueMicrotask(() => {
											result.onsuccess?.();
											finish();
										});
										return result;
									},
									add(value: unknown, key: string) {
										if (staged.has(key)) throw new Error("duplicate");
										staged.set(key, structuredClone(value));
										finish();
									},
									put(value: unknown, key: string) {
										staged.set(key, structuredClone(value));
										finish();
									},
								};
							},
						};
						function finish() {
							if (scheduled) return;
							scheduled = true;
							queueMicrotask(() => {
								if (aborted) return;
								if (failCommit) {
									failCommit = false;
									tx.abort();
									return;
								}
								stores.set(name, staged);
								tx.oncomplete?.();
							});
						}
						return tx;
					},
				},
			};
			queueMicrotask(() => request.onsuccess?.());
			return request;
		},
	},
});
Object.defineProperty(globalThis, "navigator", {
	configurable: true,
	value: {
		locks: {
			async request(
				_key: string,
				_options: unknown,
				run: (lock: object | null) => Promise<void>,
			) {
				if (lockHeld) return run(null);
				lockHeld = true;
				try {
					await run({});
				} finally {
					lockHeld = false;
				}
			},
		},
	},
});
afterAll(() => {
	if (originalIdb) Object.defineProperty(globalThis, "indexedDB", originalIdb);
	else Reflect.deleteProperty(globalThis, "indexedDB");
	if (originalNavigator)
		Object.defineProperty(globalThis, "navigator", originalNavigator);
	else Reflect.deleteProperty(globalThis, "navigator");
});
beforeEach(() => {
	stores.clear();
	failCommit = false;
	lockHeld = false;
});

function fixture(): LocalDeviceVault {
	const key = {
		kty: "OKP" as const,
		crv: "Ed25519" as const,
		x: "approved-controller",
	};
	return {
		deviceId: "device",
		grantId: "reader-grant",
		manifestJws: "signed-manifest",
		ownerControllerKey: key,
		controllerPublic: {
			device_id: "device",
			endpoint_id: "endpoint",
			controller_key: key,
			archive_key: Array(32).fill(7),
			telemetry_member: {
				endpoint_id: "endpoint",
				signing_key: { ...key, x: "endpoint-key" },
			},
		},
		controllerVault: new Uint8Array(80).fill(1),
		invitationVault: new Uint8Array(80).fill(2),
	};
}
function replacement(vault: LocalDeviceVault): LocalDeviceVault {
	return {
		...vault,
		controllerVault: new Uint8Array(80).fill(3),
		invitationVault: vault.invitationVault
			? new Uint8Array(80).fill(4)
			: undefined,
	};
}

test("password replacement commits both vaults and preserves every MLS snapshot", async () => {
	const previous = fixture();
	await addDeviceVault(scope, previous);
	stores.set(
		"snapshots",
		new Map([
			["existing", { ciphertext: "existing MLS ciphertext", revision: 19 }],
		]),
	);
	const snapshots = structuredClone(stores.get("snapshots"));
	const next = replacement(previous);
	await replaceRewrappedVault(scope, previous, next);
	expect(await readDeviceVault(scope, "device")).toEqual(next);
	expect(stores.get("snapshots")).toEqual(snapshots);
	await expect(
		replaceRewrappedVault(scope, previous, replacement(next)),
	).rejects.toThrow("vault changed");
	expect(await readDeviceVault(scope, "device")).toEqual(next);
	await expect(
		replaceRewrappedVault(
			{ ...scope, account: "another-user" },
			next,
			previous,
		),
	).rejects.toThrow("vault changed");
	expect(await readDeviceVault(scope, "device")).toEqual(next);
});

test("failed commit and stale invitation changes leave the winning envelopes intact", async () => {
	const previous = fixture();
	await addDeviceVault(scope, previous);
	failCommit = true;
	await expect(
		replaceRewrappedVault(scope, previous, replacement(previous)),
	).rejects.toThrow("not committed");
	expect(await readDeviceVault(scope, "device")).toEqual(previous);
	for (const changed of [
		{ ...previous, invitationVault: new Uint8Array(80).fill(5) },
		{ ...previous, grantId: "other-grant" },
		{
			...previous,
			ownerControllerKey: {
				...previous.controllerPublic.controller_key,
				x: "other-owner",
			},
		},
		{ ...previous, requiresFreshEndpoint: true },
	]) {
		await expect(
			replaceRewrappedVault(scope, changed, replacement(changed)),
		).rejects.toThrow("vault changed");
		expect(await readDeviceVault(scope, "device")).toEqual(previous);
	}
	for (const next of [
		{ ...replacement(previous), invitationVault: undefined },
		{ ...replacement(previous), invitationVault: previous.invitationVault },
		{
			...replacement(previous),
			controllerPublic: {
				...previous.controllerPublic,
				endpoint_id: "rotated",
			},
		},
	])
		await expect(replaceRewrappedVault(scope, previous, next)).rejects.toThrow(
			"preserve",
		);
});

test("password workflow clears both buffers on commit failure and releases its lock", async () => {
	const previous = fixture();
	await addDeviceVault(scope, previous);
	let captured: Uint8Array[] = [];
	const crypto: Pick<DeviceCrypto, "rewrapControllerVaults"> = {
		rewrapControllerVaults(device, old, next, controller, invitation) {
			expect(device).toBe("device");
			expect(controller).toEqual(previous.controllerVault);
			expect(invitation).toEqual(previous.invitationVault ?? new Uint8Array());
			expect(new TextDecoder().decode(old)).toBe("previous password");
			expect(new TextDecoder().decode(next)).toBe("replacement password");
			captured = [old, next];
			return {
				controller: {
					public_bundle: previous.controllerPublic,
					vault: Array(80).fill(3),
				},
				invitation: {
					public_key: previous.controllerPublic.controller_key,
					vault: Array(80).fill(4),
				},
			};
		},
	};
	failCommit = true;
	await expect(
		changeDevicePassword(
			scope,
			previous,
			"previous password",
			"replacement password",
			crypto,
		),
	).rejects.toThrow("not committed");
	expect(captured.every((bytes) => bytes.every((byte) => byte === 0))).toBe(
		true,
	);
	expect(await readDeviceVault(scope, "device")).toEqual(previous);
	expect(lockHeld).toBe(false);
	const next = await changeDevicePassword(
		scope,
		previous,
		"previous password",
		"replacement password",
		crypto,
	);
	expect(next.controllerPublic).toEqual(previous.controllerPublic);
	expect(captured.every((bytes) => bytes.every((byte) => byte === 0))).toBe(
		true,
	);
	await Promise.resolve();
	expect(lockHeld).toBe(false);
	lockHeld = true;
	await expect(
		changeDevicePassword(
			scope,
			next,
			"previous password",
			"replacement password",
			crypto,
		),
	).rejects.toThrow("another tab");
});

test("shared-user backup contains only encrypted keys and restores as a fresh endpoint", async () => {
	const previous = fixture();
	previous.invitationVault = undefined;
	await addDeviceVault(scope, previous);
	const next = replacement(previous);
	await replaceRewrappedVault(scope, previous, next);
	const blob = encryptedControllerBackup(scope, {
		...next,
		...{ plaintextPassword: "must not be exported" },
	});
	const text = await blob.text();
	expect(text).not.toContain("plaintextPassword");
	expect(text).not.toContain("must not be exported");
	const restored = controllerBackup(text, scope, "device");
	expect(restored.invitationVault).toBeUndefined();
	expect(restored.controllerVault).toEqual(next.controllerVault);
	expect(restored.controllerPublic).toEqual(next.controllerPublic);
	expect(restored.requiresFreshEndpoint).toBe(true);
});

test("crypto rejection, identity substitution and cancellation never replace saved keys", async () => {
	const previous = fixture();
	await addDeviceVault(scope, previous);
	let captured: Uint8Array[] = [];
	const failing: Pick<DeviceCrypto, "rewrapControllerVaults"> = {
		rewrapControllerVaults(_device, old, next) {
			captured = [old, next];
			throw new Error("Incorrect password or damaged vault");
		},
	};
	await expect(
		changeDevicePassword(
			scope,
			previous,
			"incorrect password",
			"replacement password",
			failing,
		),
	).rejects.toThrow("Incorrect password");
	expect(captured.every((bytes) => bytes.every((byte) => byte === 0))).toBe(
		true,
	);
	expect(await readDeviceVault(scope, "device")).toEqual(previous);
	const substituted: Pick<DeviceCrypto, "rewrapControllerVaults"> = {
		rewrapControllerVaults() {
			return {
				controller: {
					public_bundle: {
						...previous.controllerPublic,
						endpoint_id: "other-endpoint",
					},
					vault: Array(80).fill(3),
				},
				invitation: null,
			};
		},
	};
	await expect(
		changeDevicePassword(
			scope,
			previous,
			"previous password",
			"replacement password",
			substituted,
		),
	).rejects.toThrow("saved identities");
	expect(await readDeviceVault(scope, "device")).toEqual(previous);
	const abort = new AbortController();
	const cancelled: Pick<DeviceCrypto, "rewrapControllerVaults"> = {
		rewrapControllerVaults() {
			abort.abort(new Error("Cancelled"));
			return {
				controller: {
					public_bundle: previous.controllerPublic,
					vault: Array(80).fill(3),
				},
				invitation: {
					public_key: previous.controllerPublic.controller_key,
					vault: Array(80).fill(4),
				},
			};
		},
	};
	await expect(
		changeDevicePassword(
			scope,
			previous,
			"previous password",
			"replacement password",
			cancelled,
			abort.signal,
		),
	).rejects.toThrow("Cancelled");
	expect(await readDeviceVault(scope, "device")).toEqual(previous);
	expect(lockHeld).toBe(false);
});
