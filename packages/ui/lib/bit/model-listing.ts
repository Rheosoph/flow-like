import type { IBitState } from "../../state/backend-state/bit-state";
import type { IBit, IBitTypes } from "../schema";
import type { IBitSearchQuery } from "../schema/hub/bit-search-query";

/** The hub answers at most this many rows per search, whatever the caller asks. */
const PAGE_SIZE = 100;

/** Stop rather than page forever if the hub keeps returning full pages. */
const MAX_PAGES = 50;

/**
 * Every bit matching the query, not just the first page.
 *
 * A search without an explicit limit returns 50 rows, so a catalogue that has
 * outgrown that silently loses models — and which ones it loses is unstable,
 * because the query declares no order.
 */
export async function searchAllBits(
	this: IBitState,
	query: IBitSearchQuery,
): Promise<IBit[]> {
	const bits: IBit[] = [];

	for (let page = 0; page < MAX_PAGES; page += 1) {
		const rows = await this.searchBits({
			...query,
			limit: PAGE_SIZE,
			offset: page * PAGE_SIZE,
		});
		// A garbled page must fail the listing, not end it early or spread a string into it.
		if (!Array.isArray(rows)) {
			throw new Error(
				`Bit search page ${page} returned ${typeof rows} instead of a list`,
			);
		}
		bits.push(...rows);
		if (rows.length < PAGE_SIZE) break;
	}

	return bits;
}

/** Convenience wrapper for the common "give me every model of these types" call. */
export async function searchAllBitsOfType(
	this: IBitState,
	bitTypes: IBitTypes[],
): Promise<IBit[]> {
	return await searchAllBits.call(this, { bit_types: bitTypes });
}

function displayName(bit: IBit): string | undefined {
	const meta = bit.meta?.en ?? Object.values(bit.meta ?? {})[0];
	const name = meta?.name?.trim();
	return name ? name : undefined;
}

/**
 * Marks a model as withdrawn from the catalogue.
 *
 * A retired model keeps its full metadata, because boards that already reference
 * it still render its name and icon and still run it. The tag only removes it
 * from the lists people pick from, so no new work adopts it.
 */
export const DEPRECATED_TAG = "deprecated";

/** Whether the catalogue still offers this model for new work. */
export function isDeprecated(bit: IBit): boolean {
	const tags = Object.values(bit.meta ?? {}).flatMap(
		(meta) => meta?.tags ?? [],
	);
	return tags.some((tag) => tag.trim().toLowerCase() === DEPRECATED_TAG);
}

/**
 * The bits a person can actually choose, out of everything the hub returned.
 *
 * A model that another model lists as a dependency is a component of that pack,
 * not a product of its own: the text half of an image-embedding pair is an
 * `Embedding` bit like any other, and shows up in an embedding picker unless it
 * is filtered out here. A bit without metadata cannot be presented at all, and a
 * retired one must not be presented even though it still resolves.
 */
export function listableModels(bits: IBit[]): IBit[] {
	const componentIds = new Set<string>();
	for (const bit of bits) {
		for (const dependency of bit.dependencies ?? []) {
			const id = String(dependency).split(":").pop();
			if (id) componentIds.add(id);
		}
	}

	return bits.filter(
		(bit) =>
			!componentIds.has(bit.id) &&
			displayName(bit) !== undefined &&
			!isDeprecated(bit),
	);
}
