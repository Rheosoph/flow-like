import type { IApp } from "../../lib/schema/app/app";
import type { IMetadata } from "../../lib/schema/bit/bit-pack";

export type LibraryItem = IMetadata & { id: string; app: IApp };
export type SortMode = "recent" | "alpha";

export const COLLAPSED_ROWS = 1;
export const CARD_MIN_W_DESKTOP = 224;
export const CARD_MIN_W_MOBILE = 200;

export const CATEGORY_COLORS: Record<string, string> = {
	Business: "oklch(0.65 0.15 250)",
	Communication: "oklch(0.65 0.15 290)",
	Education: "oklch(0.65 0.15 145)",
	Entertainment: "oklch(0.65 0.15 330)",
	Finance: "oklch(0.65 0.15 160)",
	"Food And Drink": "oklch(0.65 0.15 55)",
	Games: "oklch(0.65 0.15 310)",
	Health: "oklch(0.65 0.15 145)",
	Lifestyle: "oklch(0.65 0.15 20)",
	Music: "oklch(0.65 0.15 280)",
	News: "oklch(0.65 0.15 220)",
	Other: "oklch(0.65 0.08 250)",
	Photography: "oklch(0.65 0.15 80)",
	Productivity: "oklch(0.65 0.15 240)",
	Shopping: "oklch(0.65 0.15 40)",
	Social: "oklch(0.65 0.15 200)",
	Sports: "oklch(0.65 0.15 130)",
	Travel: "oklch(0.65 0.15 180)",
	Utilities: "oklch(0.65 0.10 230)",
	Weather: "oklch(0.65 0.15 210)",
	Anime: "oklch(0.65 0.15 350)",
};

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
