import type { IApp } from "../../lib/schema/app/app";
import type { IMetadata } from "../../lib/schema/bit/bit-pack";
import type { ISystemTime } from "../../lib/schema/flow/event";

export type LibraryItem = IMetadata & { id: string; app: IApp };
export type SortMode = "recent" | "alpha";

export const COLLAPSED_ROWS = 1;
export const CARD_MIN_W_DESKTOP = 224;
export const CARD_MIN_W_MOBILE = 200;

export { CATEGORY_COLORS } from "../../lib/category-meta";

/** Apps the user never ordered sort after every app that carries an explicit rank. */
const UNRANKED = Number.MAX_SAFE_INTEGER;

/**
 * Last-resort tie-break. Every comparator below ends here so that two items can
 * never compare equal: `Array.prototype.sort` would otherwise fall back to the
 * input order, and the apps list arrives in a different order depending on
 * whether it came from local storage or from a remote sync.
 */
function byIdentity(a: LibraryItem, b: LibraryItem): number {
	const byName = (a.name ?? "").localeCompare(b.name ?? "");
	if (byName !== 0) return byName;
	return a.id.localeCompare(b.id);
}

/** Newest first. Seconds then nanos, because the pair exceeds safe integer range if combined. */
function byTimestamp(a?: ISystemTime, b?: ISystemTime): number {
	const bySecs = (b?.secs_since_epoch ?? 0) - (a?.secs_since_epoch ?? 0);
	if (bySecs !== 0) return bySecs;
	return (b?.nanos_since_epoch ?? 0) - (a?.nanos_since_epoch ?? 0);
}

/**
 * The two timestamps on a library item move for different reasons: the app's
 * tracks real work (boards, events, templates), while the metadata's only moves
 * when the name, description or artwork changes. Sorting on either alone gets
 * "Recently updated" wrong — renaming an app would not surface it, and editing
 * a board would not either. The later of the two is what a user means by when
 * they last touched a project.
 */
function lastTouched(item: LibraryItem): ISystemTime | undefined {
	return byTimestamp(item.app?.updated_at, item.updated_at) <= 0
		? item.app?.updated_at
		: item.updated_at;
}

function byRecency(a: LibraryItem, b: LibraryItem): number {
	const byTime = byTimestamp(lastTouched(a), lastTouched(b));
	if (byTime !== 0) return byTime;
	return byIdentity(a, b);
}

export function compareItems(
	a: LibraryItem,
	b: LibraryItem,
	mode: SortMode,
): number {
	return mode === "alpha" ? byIdentity(a, b) : byRecency(a, b);
}

/** Pairs an app record with its metadata into the shape the sorters expect. */
export function toLibraryItem(app: IApp, metadata: IMetadata): LibraryItem {
	return { ...metadata, id: app.id, app };
}

/**
 * Orders raw `[app, metadata]` pairs by recency, for surfaces that hand the
 * tuples straight to `AppCard` instead of building library items. Shares
 * {@link byRecency} so every "recently updated" list in the product agrees.
 */
export function sortAppPairsByRecency<
	T extends readonly [IApp, IMetadata | undefined],
>(pairs: readonly T[]): T[] {
	return [...pairs].sort(([appA, metaA], [appB, metaB]) =>
		byRecency(
			toLibraryItem(appA, metaA ?? ({} as IMetadata)),
			toLibraryItem(appB, metaB ?? ({} as IMetadata)),
		),
	);
}

export function sortItems(items: LibraryItem[], mode: SortMode): LibraryItem[] {
	return [...items].sort((a, b) => compareItems(a, b, mode));
}

/**
 * Orders a user-arranged shelf (pinned or favorites). The explicit rank wins;
 * everything the user has not dragged into place falls back to the library's
 * current sort mode so the shelf never disagrees with the grid below it.
 */
export function sortItemsByRank(
	items: LibraryItem[],
	rankOf: (id: string) => number | null | undefined,
	mode: SortMode,
): LibraryItem[] {
	return [...items].sort((a, b) => {
		const rankA = rankOf(a.id) ?? UNRANKED;
		const rankB = rankOf(b.id) ?? UNRANKED;
		if (rankA !== rankB) return rankA - rankB;
		return compareItems(a, b, mode);
	});
}
