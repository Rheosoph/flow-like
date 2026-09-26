import { afterAll, beforeEach, expect, test } from "bun:test";
import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import { base64url } from "./crypto";
import { restoreAccountRecovery, saveAccountRecovery } from "./recovery";
import {
	type LocalDeviceVault,
	addDeviceVault,
	readAccountRecoveryState,
	readDeviceVault,
	replaceRewrappedVault,
} from "./storage";
import type { AccountRecoveryWrite, DeviceCrypto } from "./types";
const scope = {
	issuer: "issuer",
	account: "reader",
	apiOrigin: "https://hub.test",
	profileId: "profile",
};
const stores = new Map<string, Map<string, unknown>>();
let failCommit = false;
let lockHeld = false;
const transactions: { stores: string[]; mode: string; durability?: string }[] =
	[];
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
					transaction(
						names: string | string[],
						mode: string,
						options?: IDBTransactionOptions,
					) {
						const selected = typeof names === "string" ? [names] : names;
						transactions.push({
							stores: selected,
							mode,
							durability: options?.durability,
						});
						const stagedStores = new Map(
							selected.map((name) => [name, new Map(stores.get(name) ?? [])]),
						);
						let aborted = false;
						let scheduled = false;
						const tx = {
							oncomplete: undefined as (() => void) | undefined,
							onabort: undefined as (() => void) | undefined,
							abort() {
								aborted = true;
								queueMicrotask(() => tx.onabort?.());
							},
							objectStore(name: string) {
								const staged = stagedStores.get(name);
								if (!staged)
									throw new Error("Store is outside this transaction");
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
								for (const [name, staged] of stagedStores)
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
	transactions.length = 0;
	failCommit = false;
	lockHeld = false;
});

function fixture(): LocalDeviceVault {
	const key = {
		kty: "OKP" as const,
		crv: "Ed25519" as const,
		x: base64url(new Uint8Array(32).fill(7)),
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

function services() {
	let remote: AccountRecoveryWrite | undefined;
	let serial = 0;
	let loseAcknowledgement = false;
	const envelopes = new Map<
		string,
		{ context: unknown; password: string; backup: unknown }
	>();
	const calls: AccountRecoveryWrite[] = [];
	const passwords: Uint8Array[] = [];
	const module: Pick<
		DeviceCrypto,
		"sealAccountRecovery" | "openAccountRecovery"
	> = {
		sealAccountRecovery(context, password, bytes) {
			passwords.push(password);
			if (new TextDecoder().decode(password) !== "correct device password")
				throw new Error("Wrong password");
			const ciphertext = new Uint8Array(80).fill(++serial);
			envelopes.set(base64url(ciphertext), {
				context: structuredClone(context),
				password: new TextDecoder().decode(password),
				backup: JSON.parse(new TextDecoder().decode(bytes)),
			});
			return {
				ciphertext: Array.from(ciphertext),
				proof_jws: `signed-${serial}`,
			};
		},
		openAccountRecovery(context, password, ciphertext) {
			passwords.push(password);
			const envelope = envelopes.get(base64url(ciphertext));
			if (
				!envelope ||
				JSON.stringify(context) !== JSON.stringify(envelope.context) ||
				new TextDecoder().decode(password) !== envelope.password
			)
				throw new Error("Wrong password or scope");
			return structuredClone(envelope.backup);
		},
	};
	const api = {
		async fetch(_profile: unknown, _path: string, options: RequestInit) {
			if (options.method === "GET") {
				if (!remote) throw new Error("No backup");
				return {
					public_key: remote.public_key,
					ciphertext: remote.ciphertext,
					revision: remote.revision,
				};
			}
			const request = JSON.parse(
				options.body as string,
			) as AccountRecoveryWrite;
			calls.push(structuredClone(request));
			if (
				remote?.revision === request.revision &&
				remote.ciphertext === request.ciphertext
			)
				return { revision: request.revision };
			if ((remote?.revision ?? 0) + 1 !== request.revision)
				throw new Error("Backup changed, reconcile it");
			remote = request;
			if (loseAcknowledgement) {
				loseAcknowledgement = false;
				throw new Error("Connection lost after commit");
			}
			return { revision: request.revision };
		},
	} as IApiState;
	const input = {
		api,
		profile: {} as IProfile,
		scope,
		deviceId: "device",
		password: "correct device password",
		crypto: module,
	};
	return {
		input,
		module,
		calls,
		passwords,
		loseNextAcknowledgement() {
			loseAcknowledgement = true;
		},
		get remote() {
			return remote;
		},
		set remote(value) {
			remote = value;
		},
	};
}

test("lost acknowledgement retries exact ciphertext after reload and validates password on unchanged backup", async () => {
	await addDeviceVault(scope, fixture());
	const server = services();
	server.loseNextAcknowledgement();
	await expect(saveAccountRecovery(server.input)).rejects.toThrow(
		"after commit",
	);
	const pending = (await readAccountRecoveryState(scope, "device")).pending;
	expect(pending?.request).toEqual(server.remote);
	expect(await saveAccountRecovery(server.input)).toBe(1);
	expect(server.calls).toHaveLength(2);
	expect(server.calls[0]).toEqual(server.calls[1]);
	expect(
		(await readAccountRecoveryState(scope, "device")).pending,
	).toBeUndefined();
	await expect(
		saveAccountRecovery({ ...server.input, password: "wrong password" }),
	).rejects.toThrow("Wrong password");
	expect(server.calls).toHaveLength(2);
	expect(
		server.passwords.every((bytes) => bytes.every((byte) => byte === 0)),
	).toBe(true);
});

test("durable staging failure cannot upload, and recovery records remain scoped to account and hub", async () => {
	await addDeviceVault(scope, fixture());
	const server = services();
	// Sealing happens after reads; inject the next commit failure at that boundary.
	const seal = server.module.sealAccountRecovery;
	server.module.sealAccountRecovery = (...args) => {
		const value = seal(...args);
		failCommit = true;
		return value;
	};
	await expect(saveAccountRecovery(server.input)).rejects.toThrow(
		"not committed",
	);
	expect(server.calls).toHaveLength(0);
	expect((await readAccountRecoveryState(scope, "device")).revision).toBe(0);
	server.module.sealAccountRecovery = seal;
	await saveAccountRecovery(server.input);
	await Promise.resolve();
	for (const changed of [
		{ ...scope, account: "other" },
		{ ...scope, apiOrigin: "https://other.test" },
	]) {
		expect((await readAccountRecoveryState(changed, "device")).revision).toBe(
			0,
		);
		await expect(
			restoreAccountRecovery({ ...server.input, scope: changed }),
		).rejects.toThrow("scope");
		expect(await readDeviceVault(changed, "device")).toBeUndefined();
	}
});

test("a second browser restores stable keys as a fresh endpoint and rejects rollback", async () => {
	const original = fixture();
	await addDeviceVault(scope, original);
	const server = services();
	await saveAccountRecovery(server.input);
	await Promise.resolve();
	const first = server.remote;
	const other = {
		...server.input,
		scope: { ...scope, profileId: "another-browser" },
	};
	const restored = await restoreAccountRecovery(other);
	await Promise.resolve();
	expect(restored.requiresFreshEndpoint).toBe(true);
	expect(restored.controllerVault).toEqual(original.controllerVault);
	expect(restored.controllerPublic).toEqual(original.controllerPublic);
	const next = {
		...original,
		controllerVault: new Uint8Array(80).fill(5),
		invitationVault: new Uint8Array(80).fill(6),
	};
	await replaceRewrappedVault(scope, original, next);
	expect(await saveAccountRecovery(server.input)).toBe(2);
	await Promise.resolve();
	await restoreAccountRecovery(other);
	await Promise.resolve();
	server.remote = first;
	await expect(restoreAccountRecovery(other)).rejects.toThrow("older");
	expect((await readAccountRecoveryState(other.scope, "device")).revision).toBe(
		2,
	);
});

test("a verified concurrent winner clears a blocked upload without replacing local keys", async () => {
	const original = fixture();
	await addDeviceVault(scope, original);
	const server = services();
	const other = {
		...server.input,
		scope: { ...scope, profileId: "another-browser" },
	};
	await addDeviceVault(other.scope, {
		...original,
		controllerVault: new Uint8Array(80).fill(8),
	});
	await saveAccountRecovery(other);
	await Promise.resolve();
	await expect(saveAccountRecovery(server.input)).rejects.toThrow("reconcile");
	expect(
		(await readAccountRecoveryState(scope, "device")).pending,
	).toBeDefined();
	await expect(
		restoreAccountRecovery({ ...server.input, password: "wrong password" }),
	).rejects.toThrow("password");
	expect(
		(await readAccountRecoveryState(scope, "device")).pending,
	).toBeDefined();
	expect(await restoreAccountRecovery(server.input)).toEqual(original);
	await Promise.resolve();
	expect(
		(await readAccountRecoveryState(scope, "device")).pending,
	).toBeUndefined();
	expect(await readDeviceVault(scope, "device")).toEqual(original);
	expect(await saveAccountRecovery(server.input)).toBe(2);
	await Promise.resolve();
});

test("a failed restore commit persists neither keys nor rollback watermark and can retry", async () => {
	const original = fixture();
	await addDeviceVault(scope, original);
	const server = services();
	await saveAccountRecovery(server.input);
	await Promise.resolve();
	const other = {
		...server.input,
		scope: { ...scope, profileId: "fresh-browser" },
	};
	const open = server.module.openAccountRecovery;
	server.module.openAccountRecovery = (...args) => {
		const value = open(...args);
		failCommit = true;
		return value;
	};
	await expect(restoreAccountRecovery(other)).rejects.toThrow("not committed");
	expect(transactions.at(-1)).toEqual({
		stores: ["vaults", "recovery"],
		mode: "readwrite",
		durability: "strict",
	});
	expect(await readDeviceVault(other.scope, "device")).toBeUndefined();
	expect(await readAccountRecoveryState(other.scope, "device")).toEqual({
		revision: 0,
	});
	server.module.openAccountRecovery = open;
	const restored = await restoreAccountRecovery(other);
	await Promise.resolve();
	expect(restored.controllerVault).toEqual(original.controllerVault);
	expect(restored.requiresFreshEndpoint).toBe(true);
	expect(await readDeviceVault(other.scope, "device")).toEqual(restored);
	expect((await readAccountRecoveryState(other.scope, "device")).revision).toBe(
		1,
	);
});

test("a failed reconciliation keeps existing local keys and pending ciphertext together", async () => {
	const original = fixture();
	await addDeviceVault(scope, original);
	const server = services();
	const other = {
		...server.input,
		scope: { ...scope, profileId: "winning-browser" },
	};
	await addDeviceVault(other.scope, {
		...original,
		controllerVault: new Uint8Array(80).fill(8),
	});
	await saveAccountRecovery(other);
	await Promise.resolve();
	await expect(saveAccountRecovery(server.input)).rejects.toThrow("reconcile");
	const before = await readAccountRecoveryState(scope, "device");
	const open = server.module.openAccountRecovery;
	server.module.openAccountRecovery = (...args) => {
		const value = open(...args);
		failCommit = true;
		return value;
	};
	await expect(restoreAccountRecovery(server.input)).rejects.toThrow(
		"not committed",
	);
	expect(await readDeviceVault(scope, "device")).toEqual(original);
	expect(await readAccountRecoveryState(scope, "device")).toEqual(before);
	server.module.openAccountRecovery = open;
	expect(await restoreAccountRecovery(server.input)).toEqual(original);
	await Promise.resolve();
	expect(await readDeviceVault(scope, "device")).toEqual(original);
	expect(
		(await readAccountRecoveryState(scope, "device")).pending,
	).toBeUndefined();
	expect((await readAccountRecoveryState(scope, "device")).revision).toBe(1);
});
