/**
 * Request-size limits shared with the data routes.
 *
 * `packages/api/src/routes/app/data/batch.rs` caps a per-prefix request at
 * `MAX_PREFIXES` and rejects anything larger. Callers should never have to know
 * that: an a2ui surface resolving its images, or a folder listing being
 * downloaded, asks for whatever it has. The storage backends therefore split
 * the request here rather than pushing the cap onto every call site.
 */

import type { IStorageItemActionResult } from "../state/backend-state/types";

/** Mirrors `MAX_PREFIXES` in packages/api/src/routes/app/data/batch.rs. */
export const MAX_STORAGE_PREFIXES_PER_REQUEST = 100;

/** Requests in flight while resolving a large selection. */
const PREFIX_BATCH_CONCURRENCY = 4;

export function chunkPrefixes(
	prefixes: readonly string[],
	size = MAX_STORAGE_PREFIXES_PER_REQUEST,
): string[][] {
	const batches: string[][] = [];
	for (let index = 0; index < prefixes.length; index += size) {
		batches.push(prefixes.slice(index, index + size));
	}
	return batches;
}

/**
 * Run `request` over `prefixes` in server-sized batches, preserving order.
 *
 * A batch that throws degrades to an `error` entry per prefix in it, so one
 * failed request cannot discard results the other batches already returned.
 */
export async function requestPrefixesInBatches(
	prefixes: readonly string[],
	request: (batch: string[]) => Promise<IStorageItemActionResult[]>,
	options: { concurrency?: number; errorMessage?: string } = {},
): Promise<IStorageItemActionResult[]> {
	if (prefixes.length === 0) return [];

	const batches = chunkPrefixes(prefixes);
	if (batches.length === 1) return request(batches[0]);

	const concurrency = Math.max(
		1,
		Math.min(options.concurrency ?? PREFIX_BATCH_CONCURRENCY, batches.length),
	);
	const errorMessage = options.errorMessage ?? "Request failed";
	const results: IStorageItemActionResult[][] = new Array(batches.length);

	let next = 0;
	const worker = async () => {
		for (;;) {
			const index = next++;
			if (index >= batches.length) return;
			try {
				results[index] = await request(batches[index]);
			} catch {
				results[index] = batches[index].map((prefix) => ({
					prefix,
					error: errorMessage,
				}));
			}
		}
	};

	await Promise.all(Array.from({ length: concurrency }, worker));
	return results.flat();
}
