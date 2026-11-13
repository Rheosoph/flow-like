import type { IInboxSealedEnvelope } from "../state/backend-state/inbox-state";
import { loadDeviceCrypto } from "./device-management/crypto";
import type { IInboxCrypto, IInboxPublicKey } from "./inbox-e2e";

export const INBOX_KEYS_DB = "flow-like-inbox-keys";
const STORE = "keys";
const LOCK = "flow-like-inbox-keys";

/** The seeds of one person on this browser, sealed under a key the page can use but not read. */
interface StoredKeys {
	wrap: CryptoKey;
	iv: Uint8Array<ArrayBuffer>;
	sealed: ArrayBuffer;
}

function database(): Promise<IDBDatabase> {
	if (!globalThis.indexedDB) {
		return Promise.reject(
			new Error(
				"Encrypted messages need browser storage (IndexedDB), which is unavailable here",
			),
		);
	}
	return new Promise((resolve, reject) => {
		const request = indexedDB.open(INBOX_KEYS_DB, 1);
		request.onupgradeneeded = () => {
			if (!request.result.objectStoreNames.contains(STORE))
				request.result.createObjectStore(STORE);
		};
		request.onsuccess = () => resolve(request.result);
		request.onerror = () =>
			reject(new Error("The inbox key store could not be opened"));
	});
}

async function stored(
	mode: IDBTransactionMode,
	act: (store: IDBObjectStore) => IDBRequest,
): Promise<unknown> {
	const db = await database();
	try {
		return await new Promise((resolve, reject) => {
			const request = act(db.transaction(STORE, mode).objectStore(STORE));
			request.onsuccess = () => resolve(request.result);
			request.onerror = () =>
				reject(new Error("The inbox key store could not be used"));
		});
	} finally {
		db.close();
	}
}

function additionalData(sub: string): Uint8Array<ArrayBuffer> {
	return new TextEncoder().encode(`flow-like-inbox-keys-v1\0${sub}`);
}

async function readSeeds(sub: string): Promise<string[]> {
	const record = (await stored("readonly", (store) => store.get(sub))) as
		| StoredKeys
		| undefined;
	if (!record) return [];
	const plain = await crypto.subtle.decrypt(
		{ name: "AES-GCM", iv: record.iv, additionalData: additionalData(sub) },
		record.wrap,
		record.sealed,
	);
	const seeds: unknown = JSON.parse(new TextDecoder().decode(plain));
	if (!Array.isArray(seeds) || !seeds.every((seed) => typeof seed === "string"))
		throw new Error("The stored inbox keys are damaged");
	return seeds;
}

async function writeSeeds(sub: string, seeds: string[]): Promise<void> {
	const wrap = await crypto.subtle.generateKey(
		{ name: "AES-GCM", length: 256 },
		false,
		["encrypt", "decrypt"],
	);
	const iv = crypto.getRandomValues(new Uint8Array(12));
	const sealed = await crypto.subtle.encrypt(
		{ name: "AES-GCM", iv, additionalData: additionalData(sub) },
		wrap,
		new TextEncoder().encode(JSON.stringify(seeds)),
	);
	const record: StoredKeys = { wrap, iv, sealed };
	await stored("readwrite", (store) => store.put(record, sub));
}

/** One tab at a time makes a key, so two tabs never publish different keys for one browser. */
async function exclusive<T>(work: () => Promise<T>): Promise<T> {
	const locks = globalThis.navigator?.locks;
	if (!locks) return work();
	return await locks.request(LOCK, work);
}

/**
 * Files of encrypted conversations, sealed and opened in the device-crypto WebAssembly module.
 * They need no inbox key (each file has its own, carried in the sealed message), so the desktop
 * uses this too instead of moving whole files through native calls.
 */
export const wasmFileCrypto: Required<
	Pick<IInboxCrypto, "sealFile" | "openFile">
> = {
	sealFile: async (conversationId, file) =>
		(await loadDeviceCrypto()).inboxSealFile(conversationId, file),
	openFile: async (conversationId, key, ciphertext) =>
		(await loadDeviceCrypto()).inboxOpenFile(conversationId, key, ciphertext),
};

/**
 * This browser's inbox keys: seeds in IndexedDB, sealed by a non-extractable AES key stored
 * beside them; sealing and opening run in the device-crypto WebAssembly module.
 */
export const browserInboxCrypto: IInboxCrypto = {
	key: (principalId) =>
		exclusive(async () => {
			const module = await loadDeviceCrypto();
			const seeds = await readSeeds(principalId);
			const active = seeds.at(-1);
			if (active) return module.inboxPublicKey(active) as IInboxPublicKey;
			const { seed, ...key } = module.inboxNewKey();
			await writeSeeds(principalId, [seed]);
			return key;
		}),
	seal: async (scope, members, body) =>
		(await loadDeviceCrypto()).inboxSeal(scope, members, body) as Record<
			string,
			IInboxSealedEnvelope
		>,
	sealSecret: async (scope, keys, value) =>
		(await loadDeviceCrypto()).inboxSealSecret(
			scope,
			keys,
			value,
		) as IInboxSealedEnvelope,
	open: async (principalId, entries) => {
		const [module, seeds] = await Promise.all([
			loadDeviceCrypto(),
			readSeeds(principalId),
		]);
		if (seeds.length === 0) return entries.map(() => null);
		return module.inboxOpen(seeds, principalId, entries);
	},
	...wasmFileCrypto,
};
