import type { IApp } from "../../lib/schema/app/app";
import type { IMetadata } from "../../lib/schema/bit/bit-pack";

export type LibraryItem = IMetadata & { id: string; app: IApp };
export type SortMode = "recent" | "alpha";

export const COLLAPSED_ROWS = 1;
export const CARD_MIN_W_DESKTOP = 224;
export const CARD_MIN_W_MOBILE = 200;

export { CATEGORY_COLORS } from "../../lib/category-meta";

export function sortItems(items: LibraryItem[], mode: SortMode): LibraryItem[] {
	if (mode === "alpha") {
		return [...items].sort((a, b) => {
			const byName = (a.name ?? "").localeCompare(b.name ?? "");
			if (byName !== 0) return byName;
			return a.id.localeCompare(b.id);
		});
	}
	return [...items].sort((a, b) => {
		const bySecs =
			(b.updated_at?.secs_since_epoch ?? 0) -
			(a.updated_at?.secs_since_epoch ?? 0);
		if (bySecs !== 0) return bySecs;
		const byNanos =
			(b.updated_at?.nanos_since_epoch ?? 0) -
			(a.updated_at?.nanos_since_epoch ?? 0);
		if (byNanos !== 0) return byNanos;
		const byName = (a.name ?? "").localeCompare(b.name ?? "");
		if (byName !== 0) return byName;
		return a.id.localeCompare(b.id);
	});
}
