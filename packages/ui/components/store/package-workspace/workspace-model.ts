import {
	PackagePermissionBits,
	isMaintainer,
	isOwner,
} from "../../../lib/permission/wasm-package-permission";
import {
	type PackageMeta,
	PackageStatus,
	type PackageSummary,
	type PackageUser,
	type PackageVersion,
	type RegistryEntry,
} from "../../../lib/schema/wasm";
import { packageStoreHref } from "../package-navigation";
import type { WorkspaceTab } from "./workspace-href";

export type WorkspaceRemoteStatus =
	| "idle"
	| "loading"
	| "error"
	| "forbidden"
	| "not_found"
	| "ok";

export type WorkspaceRemoteSource = "registry" | "local";

/**
 * `unknown` is auth work in flight, `expired` a settled session whose token ran
 * out: nothing is fetched with it, and it only ends when the user signs in.
 */
export type WorkspaceAuthState =
	| "unknown"
	| "signed_in"
	| "signed_out"
	| "expired";

export type WorkspaceBanner =
	| "id_taken"
	| "registry_unavailable"
	| "signed_out"
	| "session_expired";

export interface WorkspaceAccessInput {
	packageId?: string | null;
	hasLocal: boolean;
	/** The host is still resolving a checkout, so `hasLocal` may flip to true. */
	localPending?: boolean;
	auth: WorkspaceAuthState;
	remote: {
		status: WorkspaceRemoteStatus;
		source?: WorkspaceRemoteSource;
		permission?: number;
	};
}

export interface WorkspaceAuthInput {
	readonly isLoading?: boolean;
	readonly activeNavigator?: string;
	readonly user?: { readonly expired?: boolean } | null;
}

/**
 * `signedIn` is the host's last push (`isAuthenticated && access_token`).
 * `unknown` only while something is in flight: nothing pushed yet, OIDC
 * loading, a navigator (silent renew, redirect) running, or a fresh user the
 * host has not pushed. A settled user whose token ran out is `expired`: its
 * token reads as anonymous on the server, and `user.expired` is a clock check
 * the host never re-pushes, so waiting for it would never end.
 */
export function workspaceAuthState(
	signedIn: boolean | undefined,
	auth: WorkspaceAuthInput | null | undefined,
): WorkspaceAuthState {
	const expired = auth?.user?.expired === true;
	if (signedIn === true && !expired) return "signed_in";
	if (auth?.isLoading || auth?.activeNavigator) return "unknown";
	if (expired) return signedIn === undefined ? "unknown" : "expired";
	if (auth?.user) return "unknown";
	if (signedIn === false) return "signed_out";
	return auth?.isLoading === false ? "signed_out" : "unknown";
}

export type WorkspaceAccess =
	| { mode: "loading" }
	| { mode: "not_found" }
	| { mode: "error" }
	| { mode: "session_expired" }
	| { mode: "local_only" }
	| { mode: "local"; banner: WorkspaceBanner }
	| { mode: "full"; isOwner: boolean; isMaintainer: true }
	| { mode: "redirect"; href: string };

/**
 * The only place the maintainer gate lives. It fails closed: the owner shell
 * needs a positive maintainer bit on a registry entry the server confirmed for
 * this caller, never a loading, anonymous or failed response. That confirmed
 * answer outlives a token renewal or expiry, so the shell never unmounts under
 * a maintainer mid-session. A signed-out caller holds no maintainer bit: without
 * a checkout it goes to the store page before anything is fetched, and with one
 * the registry tabs wait for a sign-in instead of guessing who owns the id.
 */
export function workspaceAccess({
	packageId,
	hasLocal,
	localPending = false,
	auth,
	remote,
}: WorkspaceAccessInput): WorkspaceAccess {
	const unpublished: WorkspaceAccess = hasLocal
		? { mode: "local_only" }
		: { mode: "not_found" };
	const sessionExpired: WorkspaceAccess = hasLocal
		? { mode: "local", banner: "session_expired" }
		: { mode: "session_expired" };
	const permission = remote.permission ?? 0;

	if (localPending) return { mode: "loading" };
	if (!packageId) return unpublished;
	if (
		auth !== "signed_out" &&
		remote.status === "ok" &&
		remote.source === "registry" &&
		isMaintainer(permission)
	) {
		return { mode: "full", isOwner: isOwner(permission), isMaintainer: true };
	}
	if (auth === "unknown") return { mode: "loading" };
	if (auth === "signed_out" && !hasLocal) {
		return { mode: "redirect", href: packageStoreHref({ id: packageId }) };
	}
	if (remote.status === "idle" || remote.status === "loading") {
		return auth === "expired" ? sessionExpired : { mode: "loading" };
	}
	if (remote.status === "ok" && remote.source !== "registry")
		return unpublished;
	if (remote.status === "not_found") return unpublished;
	if (remote.status === "error") {
		if (auth === "expired") return sessionExpired;
		return hasLocal
			? { mode: "local", banner: "registry_unavailable" }
			: { mode: "error" };
	}
	if (auth === "signed_out") return { mode: "local", banner: "signed_out" };
	if (remote.status === "forbidden") {
		return hasLocal
			? { mode: "local", banner: "id_taken" }
			: { mode: "not_found" };
	}
	if (hasLocal) return { mode: "local", banner: "id_taken" };
	return { mode: "redirect", href: packageStoreHref({ id: packageId }) };
}

export function errorHttpStatus(error: unknown): number | undefined {
	if (typeof error !== "object" || error === null || !("status" in error))
		return undefined;
	const { status } = error as { status: unknown };
	return typeof status === "number" ? status : undefined;
}

/**
 * A failed background refetch keeps a confirmed answer: only 403 and 404 are
 * authoritative enough to take it away, so a blip never tears down the shell.
 */
export function remoteStatusOf(query: {
	status: "pending" | "error" | "success";
	fetchStatus: "fetching" | "paused" | "idle";
	error: unknown;
	hasConfirmedData?: boolean;
}): WorkspaceRemoteStatus {
	if (query.status === "success") return "ok";
	if (query.status === "pending")
		return query.fetchStatus === "idle" ? "idle" : "loading";
	const status = errorHttpStatus(query.error);
	if (status === 404) return "not_found";
	if (status === 403) return "forbidden";
	return query.hasConfirmedData ? "ok" : "error";
}

export interface WorkspaceTabsInput {
	hasLocal: boolean;
	hasRemote: boolean;
	isOwner: boolean;
	isMaintainer: boolean;
	/** The host can link a checkout (desktop), so Test and Manifest show a "Link folder…" call to action. */
	canLinkLocal?: boolean;
}

export function workspaceTabs({
	hasLocal,
	hasRemote,
	isOwner,
	isMaintainer,
	canLinkLocal = false,
}: WorkspaceTabsInput): WorkspaceTab[] {
	const tabs: WorkspaceTab[] = ["overview", "nodes"];
	if (hasLocal || canLinkLocal) tabs.push("test", "manifest");
	if (hasRemote && (isOwner || isMaintainer))
		tabs.push("listing", "access", "releases");
	return tabs;
}

export type ListingField = "thumbnail" | "description" | "keywords";

export interface ListingHealth {
	missing: ListingField[];
	complete: boolean;
}

function hasText(value: string | null | undefined): boolean {
	return typeof value === "string" && value.trim().length > 0;
}

export function listingHealth(
	pkg: Pick<RegistryEntry, "manifest">,
	meta: PackageMeta | null | undefined,
): ListingHealth {
	const keywords = meta?.tags?.length
		? meta.tags
		: (pkg.manifest?.keywords ?? []);
	const checks: Array<[ListingField, boolean]> = [
		["thumbnail", hasText(meta?.thumbnail)],
		[
			"description",
			hasText(meta?.description) || hasText(pkg.manifest?.description),
		],
		["keywords", keywords.some(hasText)],
	];
	const missing = checks.flatMap(([field, ok]) => (ok ? [] : [field]));
	return { missing, complete: missing.length === 0 };
}

export type OverviewCheckStatus = "ok" | "warning" | "error" | "pending";

export type OverviewCheck =
	| {
			id: "build";
			status: OverviewCheckStatus;
			sizeBytes?: number;
			builtAt?: string | number;
	  }
	| {
			id: "lint";
			status: OverviewCheckStatus;
			errors: number;
			warnings: number;
	  };

export interface OverviewChecksInput {
	build?: {
		exists: boolean;
		failed?: boolean;
		stale?: boolean;
		sizeBytes?: number;
		builtAt?: string | number;
	} | null;
	lint?: { errors: number; warnings: number } | null;
}

/** Build and Lint only: packages have no test runner yet. */
export function overviewChecks({
	build,
	lint,
}: OverviewChecksInput): OverviewCheck[] {
	const buildStatus: OverviewCheckStatus = !build
		? "pending"
		: build.failed || !build.exists
			? "error"
			: build.stale
				? "warning"
				: "ok";
	const lintStatus: OverviewCheckStatus = !lint
		? "pending"
		: lint.errors > 0
			? "error"
			: lint.warnings > 0
				? "warning"
				: "ok";
	return [
		{
			id: "build",
			status: buildStatus,
			sizeBytes: build?.sizeBytes,
			builtAt: build?.builtAt,
		},
		{
			id: "lint",
			status: lintStatus,
			errors: lint?.errors ?? 0,
			warnings: lint?.warnings ?? 0,
		},
	];
}

function isLiveStatus(status: PackageStatus | undefined): boolean {
	return (
		status === undefined ||
		status === PackageStatus.Active ||
		status === PackageStatus.Deprecated
	);
}

/** Versions arrive newest first. */
export function liveVersion(
	versions: readonly PackageVersion[],
): PackageVersion | undefined {
	return versions.find((v) => !v.yanked && isLiveStatus(v.status));
}

export function pendingVersion(
	versions: readonly PackageVersion[],
): PackageVersion | undefined {
	return versions.find((v) => v.status === PackageStatus.PendingReview);
}

export type WorkspaceRegistryState =
	| "live"
	| "in_review"
	| "disabled"
	| "rejected";

export function registryState(
	pkg: Pick<RegistryEntry, "status" | "versions">,
): WorkspaceRegistryState {
	if (pkg.status === PackageStatus.Disabled) return "disabled";
	if (pkg.status === PackageStatus.Rejected) return "rejected";
	if (
		pkg.status === PackageStatus.PendingReview ||
		!liveVersion(pkg.versions)
	) {
		return "in_review";
	}
	return "live";
}

export interface PeopleCounts {
	owners: number;
	maintainers: number;
	users: number;
	buyers: number;
}

/** Each person counts once, at their highest role. */
export function peopleCounts(users: readonly PackageUser[]): PeopleCounts {
	const counts: PeopleCounts = {
		owners: 0,
		maintainers: 0,
		users: 0,
		buyers: 0,
	};
	for (const user of users) {
		const permission = user.permission ?? 0;
		if (isOwner(permission)) counts.owners += 1;
		else if (permission & PackagePermissionBits.Maintainer)
			counts.maintainers += 1;
		else if (permission & PackagePermissionBits.User) counts.users += 1;
		else if (permission & PackagePermissionBits.Buyer) counts.buyers += 1;
	}
	return counts;
}

/** The store card as Explore would render it from the saved listing. */
export function storePreviewSummary(
	pkg: RegistryEntry,
	meta: PackageMeta | null | undefined,
	capabilities?: string[],
): PackageSummary {
	const manifest = pkg.manifest;
	const name = hasText(meta?.name) ? (meta?.name ?? "") : manifest.name;
	const description = hasText(meta?.description)
		? (meta?.description ?? "")
		: manifest.description;
	const live = liveVersion(pkg.versions) ?? pkg.versions[0];
	return {
		id: pkg.id,
		name,
		description,
		latestVersion: live?.version ?? manifest.version,
		downloadCount: pkg.downloadCount ?? 0,
		status: pkg.status,
		keywords: meta?.tags?.length ? meta.tags : manifest.keywords,
		verified: pkg.verified,
		price: pkg.price ?? 0,
		visibility: pkg.visibility,
		primaryCategory: manifest.primaryCategory,
		secondaryCategory: manifest.secondaryCategory,
		metadata: meta
			? {
					lang: meta.lang,
					name,
					description,
					icon: meta.icon,
					thumbnail: meta.thumbnail,
				}
			: undefined,
		avgRating: pkg.avgRating,
		ratingCount: pkg.ratingCount,
		capabilities,
	};
}

/** `long_running` → `Long running`. */
export function humanizeKey(value: string | undefined): string | undefined {
	if (!value) return undefined;
	const words = value.replaceAll("_", " ").trim();
	return words.charAt(0).toUpperCase() + words.slice(1);
}
