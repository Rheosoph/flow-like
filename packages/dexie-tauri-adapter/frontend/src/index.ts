import Dexie, { type DBCore, type DBCoreMutateRequest } from "dexie";
import { invoke } from "@tauri-apps/api/core";

const BLOB_MARKER = "__fl_blob__";
const PLUGIN_PREFIX = "plugin:flow-like-dexie-blob-offload|";

interface BlobRef {
	hash: string;
	mac: string;
}

interface BlobEntry {
	key: string;
	data: number[];
}

interface BlobRefEntry {
	key: string;
	blob_ref: BlobRef;
}

function isBlobMarker(value: unknown): value is { __fl_blob__: BlobRef } {
	if (typeof value !== "object" || value === null) return false;
	const inner = (value as Record<string, unknown>)[BLOB_MARKER];
	return (
		typeof inner === "object" &&
		inner !== null &&
		typeof (inner as BlobRef).hash === "string" &&
		typeof (inner as BlobRef).mac === "string"
	);
}

function isLargeString(value: unknown, threshold: number): value is string {
	return typeof value === "string" && value.length > threshold;
}

function isLargeNumberArray(
	value: unknown,
	threshold: number,
): value is number[] {
	return (
		Array.isArray(value) &&
		value.length > threshold &&
		typeof value[0] === "number"
	);
}

function encoder(): TextEncoder {
	return new TextEncoder();
}

/** Collect all blob marker hashes from a deep object (including arrays). */
function collectAllBlobHashes(obj: unknown, result: Set<string>): void {
	if (obj === null || obj === undefined) return;
	if (isBlobMarker(obj)) {
		result.add(obj[BLOB_MARKER].hash);
		return;
	}
	if (Array.isArray(obj)) {
		for (const item of obj) collectAllBlobHashes(item, result);
		return;
	}
	if (typeof obj === "object") {
		for (const value of Object.values(obj as Record<string, unknown>)) {
			collectAllBlobHashes(value, result);
		}
	}
}

/** Increment reference counts for the given hashes (fire-and-forget). */
function incRefs(hashes: string[]): void {
	if (hashes.length === 0) return;
	invoke(`${PLUGIN_PREFIX}blob_inc_refs`, { hashes }).catch(() => {});
}

/** Decrement reference counts. Blobs reaching 0 are deleted by the backend (fire-and-forget). */
function decRefs(hashes: string[]): void {
	if (hashes.length === 0) return;
	invoke(`${PLUGIN_PREFIX}blob_dec_refs`, { hashes }).catch(() => {});
}

async function extractBlobsDeep(
	obj: unknown,
	threshold: number,
	pendingBlobs: Map<string, number[]>,
	path: string,
): Promise<unknown> {
	if (obj === null || obj === undefined) return obj;

	if (isLargeString(obj, threshold)) {
		const bytes = Array.from(encoder().encode(obj));
		const key = path || "root";
		pendingBlobs.set(key, bytes);
		return { [BLOB_MARKER]: { __pending: key } };
	}

	if (isLargeNumberArray(obj, threshold)) {
		const key = path || "root";
		pendingBlobs.set(key, obj);
		return { [BLOB_MARKER]: { __pending: key } };
	}

	if (Array.isArray(obj)) {
		const result = [];
		for (let i = 0; i < obj.length; i++) {
			result.push(
				await extractBlobsDeep(
					obj[i],
					threshold,
					pendingBlobs,
					`${path}[${i}]`,
				),
			);
		}
		return result;
	}

	if (typeof obj === "object") {
		const clone: Record<string, unknown> = {};
		for (const [key, value] of Object.entries(
			obj as Record<string, unknown>,
		)) {
			clone[key] = await extractBlobsDeep(
				value,
				threshold,
				pendingBlobs,
				path ? `${path}.${key}` : key,
			);
		}
		return clone;
	}

	return obj;
}

function resolvePending(obj: unknown, refMap: Map<string, BlobRef>): unknown {
	if (obj === null || obj === undefined) return obj;

	if (typeof obj === "object" && !Array.isArray(obj)) {
		const inner = (obj as Record<string, unknown>)[BLOB_MARKER];
		if (
			typeof inner === "object" &&
			inner !== null &&
			typeof (inner as Record<string, unknown>).__pending === "string"
		) {
			const key = (inner as Record<string, string>).__pending;
			const ref_ = refMap.get(key);
			if (ref_) return { [BLOB_MARKER]: ref_ };
		}

		const clone: Record<string, unknown> = {};
		for (const [key, value] of Object.entries(
			obj as Record<string, unknown>,
		)) {
			clone[key] = resolvePending(value, refMap);
		}
		return clone;
	}

	if (Array.isArray(obj)) {
		return obj.map((item) => resolvePending(item, refMap));
	}

	return obj;
}

async function extractBlobs(
	obj: unknown,
	threshold: number,
): Promise<unknown> {
	const pendingBlobs = new Map<string, number[]>();
	const extracted = await extractBlobsDeep(obj, threshold, pendingBlobs, "");

	if (pendingBlobs.size === 0) return extracted;

	if (pendingBlobs.size === 1) {
		const [key, data] = pendingBlobs.entries().next().value!;
		const blobRef = await invoke<BlobRef>(`${PLUGIN_PREFIX}blob_store`, {
			data,
		});
		const refMap = new Map<string, BlobRef>();
		refMap.set(key, blobRef);
		return resolvePending(extracted, refMap);
	}

	const entries: BlobEntry[] = [];
	for (const [key, data] of pendingBlobs) {
		entries.push({ key, data });
	}
	const results = await invoke<BlobRefEntry[]>(
		`${PLUGIN_PREFIX}blob_store_batch`,
		{ entries },
	);
	const refMap = new Map<string, BlobRef>();
	for (const result of results) {
		refMap.set(result.key, result.blob_ref);
	}
	return resolvePending(extracted, refMap);
}

function tryDecodeUtf8(data: number[]): string | number[] {
	try {
		return new TextDecoder("utf-8", { fatal: true }).decode(
			new Uint8Array(data),
		);
	} catch {
		return data;
	}
}

function collectBlobRefs(
	obj: Record<string, unknown>,
	path: string[],
	result: { path: string[]; ref_: BlobRef }[],
) {
	for (const [key, value] of Object.entries(obj)) {
		if (isBlobMarker(value)) {
			result.push({ path: [...path, key], ref_: value[BLOB_MARKER] });
		} else if (typeof value === "object" && value !== null) {
			collectBlobRefs(
				value as Record<string, unknown>,
				[...path, key],
				result,
			);
		}
	}
}

function setNestedValue(
	obj: Record<string, unknown>,
	path: string[],
	value: unknown,
): Record<string, unknown> {
	const clone = structuredClone(obj);
	let current: Record<string, unknown> = clone;
	for (let i = 0; i < path.length - 1; i++) {
		current = current[path[i]] as Record<string, unknown>;
	}
	current[path[path.length - 1]] = value;
	return clone;
}

async function rehydrateBlobsDeep(obj: unknown): Promise<unknown> {
	if (obj === null || obj === undefined) return obj;

	if (isBlobMarker(obj)) {
		const ref_ = obj[BLOB_MARKER];
		try {
			const data = await invoke<number[]>(`${PLUGIN_PREFIX}blob_get`, {
				hash: ref_.hash,
				mac: ref_.mac,
			});
			return tryDecodeUtf8(data);
		} catch (e) {
			console.warn("[blob-offload] blob_get failed, returning marker as-is:", e);
			return obj;
		}
	}

	if (Array.isArray(obj)) {
		return Promise.all(obj.map((item) => rehydrateBlobsDeep(item)));
	}

	if (typeof obj === "object") {
		const blobKeys: { path: string[]; ref_: BlobRef }[] = [];
		collectBlobRefs(obj as Record<string, unknown>, [], blobKeys);

		if (blobKeys.length === 0) {
			const clone: Record<string, unknown> = {};
			for (const [key, value] of Object.entries(
				obj as Record<string, unknown>,
			)) {
				clone[key] = await rehydrateBlobsDeep(value);
			}
			return clone;
		}

		if (blobKeys.length === 1) {
			const entry = blobKeys[0];
			try {
				const data = await invoke<number[]>(`${PLUGIN_PREFIX}blob_get`, {
					hash: entry.ref_.hash,
					mac: entry.ref_.mac,
				});
				return setNestedValue(
					obj as Record<string, unknown>,
					entry.path,
					tryDecodeUtf8(data),
				);
			} catch (e) {
				console.warn("[blob-offload] blob_get failed for path", entry.path, ":", e);
				return obj;
			}
		}

		const refs: BlobRefEntry[] = blobKeys.map((entry, i) => ({
			key: String(i),
			blob_ref: entry.ref_,
		}));
		try {
			const results = await invoke<BlobEntry[]>(
				`${PLUGIN_PREFIX}blob_get_batch`,
				{ refs },
			);
			let clone = structuredClone(obj) as Record<string, unknown>;
			for (let i = 0; i < results.length; i++) {
				clone = setNestedValue(clone, blobKeys[i].path, tryDecodeUtf8(results[i].data));
			}
			return clone;
		} catch (e) {
			console.warn("[blob-offload] blob_get_batch failed:", e);
			return obj;
		}
	}

	return obj;
}

/**
 * Dexie DBCore middleware that offloads large values to Tauri's native filesystem.
 *
 * Strings longer than `threshold` characters and number arrays longer than
 * `threshold` elements are transparently stored via blake3 content-addressed
 * hashing with HMAC-verified references.
 *
 * Requires the companion Tauri plugin `tauri-plugin-flow-like-dexie-blob-offload` on the Rust side.
 *
 * @param threshold - Size threshold for offloading (default: 200 chars/elements)
 */
export function dexieTauriBlobOffload(threshold = 200) {
	return {
		stack: "dbcore" as const,
		name: "flow-like-dexie-tauri-blob-offload",
		create(downcore: DBCore): DBCore {
			return {
				...downcore,
				table(name: string) {
					const table = downcore.table(name);
					const keyPath = table.schema?.primaryKey?.keyPath as string | undefined;

					return {
						...table,

						async mutate(req: DBCoreMutateRequest) {
							if (req.type === "add" || req.type === "put") {
								try {
									// Wrap old-hash collection + blob extraction in a single
									// Dexie.waitFor to prevent the IDB transaction from
									// auto-committing during the gap between steps.
									const { processed, oldHashes } = await Dexie.waitFor(
										(async () => {
											let oldHashes: Set<string> | undefined;
											if (req.type === "put" && keyPath) {
												oldHashes = new Set();
												try {
													for (const val of req.values) {
														const key = (val as Record<string, unknown>)[keyPath];
														if (key != null) {
															const existing = await table.get({ key, trans: req.trans });
															if (existing) collectAllBlobHashes(existing, oldHashes);
														}
													}
												} catch {
													/* ignore read errors for cleanup */
												}
											}

											const processed = await Promise.all(
												req.values.map((val: unknown) =>
													extractBlobs(val, threshold),
												),
											);

											return { processed, oldHashes };
										})(),
									);

									const result = await table.mutate({ ...req, values: processed });

									// Collect new blob hashes and adjust ref counts
									const newHashes = new Set<string>();
									for (const val of processed) {
										collectAllBlobHashes(val, newHashes);
									}

									const trulyNew = [...newHashes].filter((h) => !oldHashes?.has(h));
									incRefs(trulyNew);

									const stale = oldHashes
										? [...oldHashes].filter((h) => !newHashes.has(h))
										: [];
									decRefs(stale);

									return result;
								} catch (e) {
									// If the transaction died during blob extraction (common
									// during rapid streaming saves), return a synthetic result.
									// The final save will properly persist and offload blobs.
									const msg = e instanceof Error ? e.message : String(e);
									if (msg.includes("transaction") || msg.includes("InvalidState")) {
										console.debug("[blob-offload] Transaction expired during extraction, deferring to next write");
										return { numFailures: 0, failures: {}, lastResult: undefined };
									}
									throw e;
								}
							}

							// For delete: decrement refs for all blobs in deleted records
							if (req.type === "delete" && req.keys && req.keys.length > 0) {
								const deadHashes = new Set<string>();
								try {
									const records = await table.getMany({
										keys: req.keys,
										trans: req.trans,
									});
									for (const rec of records) {
										if (rec) collectAllBlobHashes(rec, deadHashes);
									}
								} catch {
									/* ignore */
								}

								const result = await table.mutate(req);

								decRefs([...deadHashes]);

								return result;
							}

							return table.mutate(req);
						},

						async get(req) {
							const result = await table.get(req);
							if (!result) return result;
							try {
								return await rehydrateBlobsDeep(result);
							} catch (e) {
								console.warn("[blob-offload] rehydration failed in get:", e);
								return result;
							}
						},

						async getMany(req) {
							const results = await table.getMany(req);
							return Promise.all(
								results.map(async (r: unknown) => {
									if (!r) return r;
									try {
										return await rehydrateBlobsDeep(r);
									} catch (e) {
										console.warn("[blob-offload] rehydration failed in getMany:", e);
										return r;
									}
								}),
							);
						},

						async query(req) {
							const result = await table.query(req);
							result.result = await Promise.all(
								result.result.map(async (r: unknown) => {
									try {
										return await rehydrateBlobsDeep(r);
									} catch (e) {
										console.warn("[blob-offload] rehydration failed in query:", e);
										return r;
									}
								}),
							);
							return result;
						},
					};
				},
			};
		},
	};
}

/**
 * Override the base storage directory used by the Rust plugin.
 * Call before any database operations to redirect blob storage
 * to a custom path (e.g. one derived from app settings).
 */
export async function configureBlobOffload(basePath: string): Promise<void> {
	await invoke(`${PLUGIN_PREFIX}blob_configure`, { basePath });
}

export type { BlobRef, BlobEntry, BlobRefEntry };
