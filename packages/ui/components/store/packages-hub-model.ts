import { PackageStatus, type PackageSummary } from "../../lib/schema/wasm";

export type PackagesHubTab = "mine" | "library" | "explore";
export type HubExploreAction = "wait" | "redirect" | "legacy";

export const PACKAGES_HUB_PATH = "/store/packages";

export const LEGACY_TAB_ALIASES: Readonly<Record<string, PackagesHubTab>> =
	Object.freeze({
		projects: "mine",
		installed: "library",
	});

export function isLegacyHubTab(param: string | null | undefined): boolean {
	return !!param && Object.hasOwn(LEGACY_TAB_ALIASES, param);
}

/** No `tab` (or an unknown one) is Explore; `?id=` is handled before the tab is read. */
export function hubTabFromParam(
	param: string | null | undefined,
): PackagesHubTab {
	if (param === "mine" || param === "library") return param;
	if (param && isLegacyHubTab(param)) return LEGACY_TAB_ALIASES[param];
	return "explore";
}

/**
 * `useExploreSupported()` is `undefined` only while pending (its auth wait is bounded) and `false` only on a hub
 * without Explore; every other error counts as supported, so Browse shows its own error state.
 */
export function hubExploreAction(
	supported: boolean | undefined,
): HubExploreAction {
	if (supported === undefined) return "wait";
	return supported ? "redirect" : "legacy";
}

export function hubHref(
	params: Pick<URLSearchParams, "toString">,
	tab: PackagesHubTab,
): string {
	const next = new URLSearchParams(params.toString());
	if (tab === "explore") {
		next.delete("tab");
	} else {
		next.set("tab", tab);
	}
	const query = next.toString();
	return query ? `${PACKAGES_HUB_PATH}?${query}` : PACKAGES_HUB_PATH;
}

export type RegistryMineState =
	| "live"
	| "in_review"
	| "rejected"
	| "disabled"
	| "deprecated";
export type RegistryMineFilter = "all" | RegistryMineState;

export const REGISTRY_MINE_FILTERS: readonly RegistryMineFilter[] = [
	"all",
	"live",
	"in_review",
	"rejected",
	"disabled",
	"deprecated",
];

export function registryMineState(
	status: PackageStatus | undefined,
): RegistryMineState {
	switch (status) {
		case PackageStatus.PendingReview:
			return "in_review";
		case PackageStatus.Rejected:
			return "rejected";
		case PackageStatus.Disabled:
		case PackageStatus.Yanked:
			return "disabled";
		case PackageStatus.Deprecated:
			return "deprecated";
		default:
			return "live";
	}
}

export function registryMineCounts(
	packages: readonly Pick<PackageSummary, "status">[],
): Record<RegistryMineFilter, number> {
	const counts = Object.fromEntries(
		REGISTRY_MINE_FILTERS.map((filter) => [filter, 0]),
	) as Record<RegistryMineFilter, number>;
	for (const pkg of packages) {
		counts.all += 1;
		counts[registryMineState(pkg.status)] += 1;
	}
	return counts;
}

export function filterRegistryMine<T extends Pick<PackageSummary, "status">>(
	packages: readonly T[],
	filter: RegistryMineFilter,
): T[] {
	if (filter === "all") return [...packages];
	return packages.filter((pkg) => registryMineState(pkg.status) === filter);
}

export type RegistryMineSort = "attention" | "name";
type RegistryMineSortable = Pick<
	PackageSummary,
	"id" | "name" | "status" | "metadata"
>;

const REGISTRY_MINE_ATTENTION: readonly RegistryMineState[] = [
	"rejected",
	"disabled",
	"in_review",
	"live",
	"deprecated",
];

function compareRegistryMineNames(
	a: RegistryMineSortable,
	b: RegistryMineSortable,
): number {
	return (
		(a.metadata?.name ?? a.name).localeCompare(
			b.metadata?.name ?? b.name,
			undefined,
			{ sensitivity: "base" },
		) || a.id.localeCompare(b.id)
	);
}

function registryMineAttention(pkg: RegistryMineSortable): number {
	return REGISTRY_MINE_ATTENTION.indexOf(registryMineState(pkg.status));
}

/** Needs attention puts rejected, disabled and in-review packages first; ties go by the name the card shows. */
export function sortRegistryMine<T extends RegistryMineSortable>(
	packages: readonly T[],
	sort: RegistryMineSort,
): T[] {
	return [...packages].sort((a, b) =>
		sort === "name"
			? compareRegistryMineNames(a, b)
			: registryMineAttention(a) - registryMineAttention(b) ||
				compareRegistryMineNames(a, b),
	);
}

/** Filter chips worth showing: All, every state present, and the active one. */
export function registryMineFilterKeys(
	counts: Record<RegistryMineFilter, number>,
	active: RegistryMineFilter,
): RegistryMineFilter[] {
	return REGISTRY_MINE_FILTERS.filter(
		(key) => key === "all" || key === active || counts[key] > 0,
	);
}
