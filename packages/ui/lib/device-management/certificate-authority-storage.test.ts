import { afterAll, beforeEach, expect, test } from "bun:test";
import type { LocalCertificateAuthority } from "./certificate-authority";
import {
	accountStorageKey,
	addCertificateAuthority,
	readCertificateAuthorities,
	replaceCertificateAuthority,
} from "./storage";

const originalIdb = Object.getOwnPropertyDescriptor(globalThis, "indexedDB");
const originalRange = Object.getOwnPropertyDescriptor(
	globalThis,
	"IDBKeyRange",
);
let persisted = new Map<string, LocalCertificateAuthority>();
let failCommit = false;
Object.defineProperty(globalThis, "IDBKeyRange", {
	configurable: true,
	value: {
		bound(lower: string, upper: string) {
			return { lower, upper };
		},
	},
});
Object.defineProperty(globalThis, "indexedDB", {
	configurable: true,
	value: {
		open(_name: string, version: number) {
			expect(version).toBe(4);
			const request = {
				result: {
					close() {},
					transaction(
						name: string,
						_mode: string,
						options?: { durability: string },
					) {
						expect(name).toBe("authorities");
						const staged = new Map(persisted);
						let scheduled = false;
						let aborted = false;
						const transaction = {
							oncomplete: undefined as (() => void) | undefined,
							onabort: undefined as (() => void) | undefined,
							abort() {
								aborted = true;
								queueMicrotask(() => transaction.onabort?.());
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
									getAll(range: { lower: string; upper: string }) {
										const result = {
											result: structuredClone(
												[...staged]
													.filter(
														([key]) => key >= range.lower && key <= range.upper,
													)
													.map(([, value]) => value),
											),
											onsuccess: undefined as (() => void) | undefined,
										};
										queueMicrotask(() => {
											result.onsuccess?.();
											finish();
										});
										return result;
									},
									add(value: LocalCertificateAuthority, key: string) {
										expect(options?.durability).toBe("strict");
										if (staged.has(key)) throw new Error("duplicate");
										staged.set(key, structuredClone(value));
										finish();
									},
									put(value: LocalCertificateAuthority, key: string) {
										expect(options?.durability).toBe("strict");
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
									transaction.abort();
									return;
								}
								persisted = staged;
								transaction.oncomplete?.();
							});
						}
						return transaction;
					},
				},
				onsuccess: undefined as (() => void) | undefined,
			};
			queueMicrotask(() => request.onsuccess?.());
			return request;
		},
	},
});
afterAll(() => {
	if (originalIdb) Object.defineProperty(globalThis, "indexedDB", originalIdb);
	else Reflect.deleteProperty(globalThis, "indexedDB");
	if (originalRange)
		Object.defineProperty(globalThis, "IDBKeyRange", originalRange);
	else Reflect.deleteProperty(globalThis, "IDBKeyRange");
});
beforeEach(() => {
	persisted = new Map();
	failCommit = false;
});
const scope = {
	issuer: "issuer",
	account: "owner",
	apiOrigin: "https://hub.test",
	profileId: "profile",
};
const id = "00000000-0000-4000-8000-000000000001";
function authority(account = scope): LocalCertificateAuthority {
	return {
		public_bundle: {
			account_binding: accountStorageKey(account),
			authority_id: id,
			label: "Org",
			dns_suffixes: ["example.com"],
			ip_addresses: [],
			root_certificate_pem: "public root",
			issuer_certificate_pem: "public intermediate",
			sha256_fingerprint: "a".repeat(64),
			not_before: 100,
			not_after: 200,
			issuer_not_after: 150,
		},
		vault: new Uint8Array(80).fill(1),
	};
}

test("authority storage excludes root backups even when the caller supplies one", async () => {
	await addCertificateAuthority(scope, {
		...authority(),
		root_vault: "offline-root-backup",
	} as LocalCertificateAuthority);
	expect(JSON.stringify([...persisted.values()])).not.toContain(
		"offline-root-backup",
	);
	expect(await readCertificateAuthorities(scope)).toEqual([authority()]);
});

test("local authority namespaces isolate account, hub, issuer and profile", async () => {
	await addCertificateAuthority(scope, authority());
	for (const other of [
		{ ...scope, account: "other" },
		{ ...scope, apiOrigin: "https://other.test" },
		{ ...scope, issuer: "other" },
		{ ...scope, profileId: "other" },
	]) {
		expect(await readCertificateAuthorities(other)).toEqual([]);
		await expect(addCertificateAuthority(other, authority())).rejects.toThrow(
			"this account",
		);
		await addCertificateAuthority(other, authority(other));
		expect(await readCertificateAuthorities(other)).toEqual([authority(other)]);
	}
	expect(await readCertificateAuthorities(scope)).toEqual([authority()]);
});

test("renewal commits atomically and rejects stale tabs or changed roots", async () => {
	const previous = authority();
	await addCertificateAuthority(scope, previous);
	const next = { ...previous, vault: new Uint8Array(80).fill(2) };
	failCommit = true;
	await expect(
		replaceCertificateAuthority(scope, previous, next),
	).rejects.toThrow("not committed");
	expect(await readCertificateAuthorities(scope)).toEqual([previous]);
	await replaceCertificateAuthority(scope, previous, {
		...next,
		root_vault: "must-not-persist",
	} as LocalCertificateAuthority);
	expect(JSON.stringify([...persisted.values()])).not.toContain(
		"must-not-persist",
	);
	await expect(
		replaceCertificateAuthority(scope, previous, next),
	).rejects.toThrow("local authority changed");
	await expect(
		replaceCertificateAuthority(scope, next, {
			...next,
			public_bundle: { ...next.public_bundle, root_certificate_pem: "other" },
		}),
	).rejects.toThrow("saved root");
});
