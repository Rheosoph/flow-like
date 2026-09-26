import {
	hasElevatedAccess,
	readManifestAccess,
} from "../../../lib/app-package-overview";
import {
	WIDGET_NET_CAPABILITY,
	readWidgetPurposes,
} from "../../../lib/package-capabilities";
import { readManifestWidgets } from "../../../lib/package-widgets";
import { PackagePermissionBits } from "../../../lib/permission/wasm-package-permission";
import {
	type InstalledPackage,
	PackageStatus,
	type PackageSummary,
	type PackageUpdate,
} from "../../../lib/schema/wasm";
import { packageStoreHref } from "../package-navigation";

/** `unknown`: installed, but no registry row says how the viewer holds it (e.g. a paid package seen anonymously). */
export type LibraryAccess =
	| "bought"
	| "shared"
	| "owner"
	| "maintainer"
	| "free"
	| "local"
	| "unknown";
export type LibraryState = "not_installed" | "ready" | "update" | "problem";

export interface LibraryEntry {
	id: string;
	/** Registry listing: the access row, or a lookup for an installed package. */
	summary?: PackageSummary;
	installed?: InstalledPackage;
	update?: PackageUpdate;
	access: LibraryAccess;
	state: LibraryState;
	name: string;
	searchText: string;
}

export interface LibraryInput {
	/** `access=library` rows: bought (Buyer) or shared (User). */
	library: readonly PackageSummary[];
	installed: readonly InstalledPackage[];
	updates: readonly PackageUpdate[];
	/** Summaries for installed packages without an access row. */
	lookups: readonly PackageSummary[];
	/** Compile status on this machine; `error` marks a problem. */
	statusOf?: (id: string) => string | undefined;
}

export const LIBRARY_STATE_FILTERS = [
	"all",
	"not_installed",
	"ready",
	"update",
	"problem",
] as const;
export type LibraryStateFilter = (typeof LIBRARY_STATE_FILTERS)[number];

export const LIBRARY_ACCESS_FILTERS = [
	"all",
	"bought",
	"shared",
	"owner",
	"maintainer",
	"free",
	"local",
] as const;
export type LibraryAccessFilter = (typeof LIBRARY_ACCESS_FILTERS)[number];

export interface LibraryFilter {
	state: LibraryStateFilter;
	access: LibraryAccessFilter;
}

export interface LibraryCounts {
	state: Record<LibraryStateFilter, number>;
	access: Record<LibraryAccessFilter, number>;
}

export interface UpdateConsent {
	needsConsent: boolean;
	/** Capabilities the latest version declares that the installed one does not. */
	newTags: string[];
}

/**
 * `checking`: the registry summary is still loading. `unverified`: it settled
 * without a summary of the offered version, so its permissions are unknown.
 */
export type LibraryUpdateCheck =
	| { status: "none" }
	| { status: "checking" }
	| { status: "unverified" }
	| ({ status: "verified" } & UpdateConsent);

/** The slice of `react-oidc-context`'s state that says whether sign-in has settled. */
export interface LibraryAuthState {
	readonly isLoading?: boolean;
	readonly activeNavigator?: string;
	readonly isAuthenticated?: boolean;
}

const LOOKUP_LIMIT = 100;

/**
 * Hosts push `signedIn` while OIDC is still loading (as `false`), and a
 * desktop without an OIDC config never pushes it. `undefined` = not known yet.
 */
export function resolveLibrarySignedIn(
	signedIn: boolean | undefined,
	auth: LibraryAuthState | null | undefined,
): boolean | undefined {
	if (signedIn === true) return true;
	if (auth?.isLoading || auth?.activeNavigator) return undefined;
	if (signedIn === false) return false;
	return auth?.isLoading === false && !auth.isAuthenticated ? false : undefined;
}

export function libraryStoreHref(id: string): string {
	return packageStoreHref({ id, tab: "library" });
}

function isLocal(installed: InstalledPackage | undefined): boolean {
	return installed?.source?.type === "local";
}

function accessOf(
	summary: PackageSummary | undefined,
	inLibrary: boolean,
	installed: InstalledPackage | undefined,
): LibraryAccess {
	const permission = summary?.viewerPermission ?? 0;
	if (permission & PackagePermissionBits.Buyer) return "bought";
	if (permission & PackagePermissionBits.User) return "shared";
	if (permission & PackagePermissionBits.Owner) return "owner";
	if (permission & PackagePermissionBits.Maintainer) return "maintainer";
	if (inLibrary && summary) return summary.price > 0 ? "bought" : "shared";
	if (isLocal(installed)) return "local";
	return summary?.price === 0 ? "free" : "unknown";
}

function stateOf(
	installed: InstalledPackage | undefined,
	update: PackageUpdate | undefined,
	status: string | undefined,
): LibraryState {
	if (status === "error") return "problem";
	if (!installed) return "not_installed";
	return update ? "update" : "ready";
}

/** Installed registry packages the library listing does not describe; local `.wasm` installs have no registry counterpart. */
export function libraryIdsToLookUp(
	library: readonly PackageSummary[],
	installed: readonly InstalledPackage[],
): string[] {
	const listed = new Set(library.map((pkg) => pkg.id));
	return [
		...new Set(
			installed
				.filter((pkg) => !isLocal(pkg) && !listed.has(pkg.id))
				.map((pkg) => pkg.id),
		),
	]
		.sort()
		.slice(0, LOOKUP_LIMIT);
}

export function mergeLibrary({
	library,
	installed,
	updates,
	lookups,
	statusOf,
}: LibraryInput): LibraryEntry[] {
	const listed = new Map(library.map((pkg) => [pkg.id, pkg]));
	const lookedUp = new Map(lookups.map((pkg) => [pkg.id, pkg]));
	const installedById = new Map(installed.map((pkg) => [pkg.id, pkg]));
	const updatesById = new Map(
		updates.map((update) => [update.packageId, update]),
	);
	const ids = new Set([...listed.keys(), ...installedById.keys()]);

	return [...ids]
		.map((id): LibraryEntry => {
			const pkg = installedById.get(id);
			const summary =
				listed.get(id) ?? (isLocal(pkg) ? undefined : lookedUp.get(id));
			const update = pkg && !isLocal(pkg) ? updatesById.get(id) : undefined;
			const name =
				summary?.metadata?.name ??
				summary?.name ??
				pkg?.metadata?.name ??
				pkg?.manifest?.name ??
				id;
			const description =
				summary?.metadata?.description ??
				summary?.description ??
				pkg?.metadata?.description ??
				pkg?.manifest?.description ??
				"";
			const keywords = summary?.keywords ?? pkg?.manifest?.keywords ?? [];
			return {
				id,
				summary,
				installed: pkg,
				update,
				access: accessOf(summary, listed.has(id), pkg),
				state: stateOf(pkg, update, statusOf?.(id)),
				name,
				searchText: [description, ...keywords].join(" "),
			};
		})
		.sort(
			(left, right) =>
				left.name.localeCompare(right.name) || left.id.localeCompare(right.id),
		);
}

export function matchesLibraryFilter(
	entry: LibraryEntry,
	filter: LibraryFilter,
): boolean {
	return (
		(filter.state === "all" || entry.state === filter.state) &&
		(filter.access === "all" || entry.access === filter.access)
	);
}

function zeroCounts<K extends string>(keys: readonly K[]): Record<K, number> {
	return Object.fromEntries(keys.map((key) => [key, 0])) as Record<K, number>;
}

/** `unknown` access has no chip of its own; those entries count only under "all". */
export function libraryCounts(entries: readonly LibraryEntry[]): LibraryCounts {
	const counts: LibraryCounts = {
		state: zeroCounts(LIBRARY_STATE_FILTERS),
		access: zeroCounts(LIBRARY_ACCESS_FILTERS),
	};
	for (const entry of entries) {
		counts.state[entry.state] += 1;
		if (entry.access !== "unknown") counts.access[entry.access] += 1;
	}
	counts.state.all = entries.length;
	counts.access.all = entries.length;
	return counts;
}

export function filterLibrary(
	entries: readonly LibraryEntry[],
	filter: LibraryFilter,
): LibraryEntry[] {
	return entries.filter((entry) => matchesLibraryFilter(entry, filter));
}

/** Mirror of the hub's `widgets_declare_network_access`: a widget purpose with a source or a network input. */
function widgetsDeclareNetwork(manifest: unknown): boolean {
	return readManifestWidgets(manifest).some(
		(widget) => readWidgetPurposes(widget.contract).length > 0,
	);
}

/** The installed version's tags in the vocabulary of `PackageSummary.capabilities`. */
export function installedCapabilityTags(manifest: unknown): string[] {
	const tags = readManifestAccess(manifest)?.capabilityTags ?? [];
	return widgetsDeclareNetwork(manifest)
		? [...tags, WIDGET_NET_CAPABILITY]
		: tags;
}

/**
 * `latest` undefined means an older registry that sends no capabilities. Any
 * new tag outside the runtime group (network, files, data, accounts, models)
 * needs consent — the same line `hasElevatedAccess` draws everywhere else.
 */
export function updateConsent({
	installed,
	latest,
}: {
	installed: Pick<InstalledPackage, "manifest"> | undefined;
	latest: readonly string[] | undefined;
}): UpdateConsent {
	if (!installed || !latest) return { needsConsent: false, newTags: [] };
	const current = new Set(installedCapabilityTags(installed.manifest));
	const newTags = [...new Set(latest)].filter((tag) => !current.has(tag));
	return {
		needsConsent: hasElevatedAccess(newTags),
		newTags,
	};
}

/** Fails closed: only a registry summary of the offered version can say which permissions it adds. */
export function libraryUpdateCheck(
	entry: Pick<LibraryEntry, "installed" | "update" | "summary">,
	summariesPending: boolean,
): LibraryUpdateCheck {
	const { installed, update, summary } = entry;
	if (!installed || !update) return { status: "none" };
	if (summary && summary.latestVersion === update.latestVersion) {
		return {
			status: "verified",
			...updateConsent({ installed, latest: summary.capabilities }),
		};
	}
	return { status: summariesPending ? "checking" : "unverified" };
}

/** Card data for entries the registry does not describe, e.g. a local `.wasm`. */
export function librarySummary(entry: LibraryEntry): PackageSummary {
	if (entry.summary) return entry.summary;
	const manifest = entry.installed?.manifest;
	return {
		id: entry.id,
		name: entry.name,
		description: manifest?.description ?? "",
		latestVersion: entry.installed?.version ?? manifest?.version ?? "",
		downloadCount: 0,
		status: PackageStatus.Active,
		keywords: manifest?.keywords ?? [],
		verified: false,
		price: 0,
		visibility: "public",
		metadata: entry.installed?.metadata,
		capabilities: manifest ? installedCapabilityTags(manifest) : undefined,
	};
}
