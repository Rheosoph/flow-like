"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
	AlertCircle,
	ArrowRight,
	Code2,
	Download,
	ExternalLink,
	History,
	type LucideIcon,
	MoreHorizontal,
	Package,
	RefreshCw,
	ShieldCheck,
	Sparkles,
	Users,
} from "lucide-react";
import Link from "next/link";
import { type ReactNode, useCallback, useMemo, useState } from "react";
import { useSearch } from "../../hooks/use-search-index";
import { isOwner } from "../../lib/permission/wasm-package-permission";
import { asArray } from "../../lib/response-shape";
import type { PackageSummary } from "../../lib/schema/wasm";
import { cn } from "../../lib/utils";
import {
	useAuthStatusStore,
	useBackend,
	useBackendReady,
} from "../../state/backend-state";
import { Button } from "../ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../ui/dropdown-menu";
import { EmptyState } from "../ui/empty-state";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";
import { exploreSearchHref } from "./explore/explore-href";
import {
	PackageCard,
	PackageCardStat,
	PackageCardStats,
	PackageRatingStar,
	formatCompact,
} from "./package-card";
import { resolveLibrarySignedIn } from "./package-library/library-model";
import type { LibraryAuth } from "./package-library/use-library-packages";
import { packageStoreHref } from "./package-navigation";
import {
	type WorkspaceTab,
	packageWorkspaceHref,
} from "./package-workspace/workspace-href";
import {
	HubFilterChips,
	HubNoMatches,
	HubRetryAlert,
	HubSearchToolbar,
	HubSignInPrompt,
	HubStatePill,
	type HubTone,
} from "./packages-hub-controls";
import { PackagesHubLayout } from "./packages-hub-layout";
import {
	type RegistryMineFilter,
	type RegistryMineSort,
	type RegistryMineState,
	filterRegistryMine,
	registryMineCounts,
	registryMineFilterKeys,
	registryMineState,
	sortRegistryMine,
} from "./packages-hub-model";
import {
	PACKAGE_GRID_CLASS_NAME,
	PackageCardSkeleton,
} from "./packages-store-page";

export interface RegistryMinePackagesProps {
	auth: LibraryAuth;
	navigation?: ReactNode;
}

export type RegistryMineStatus = "loading" | "signed-out" | "error" | "ready";

const MAINTAINER_LIMIT = 100;
const REGISTRY_STALE_MS = 60_000;
const DESKTOP_DOWNLOAD_URL = "https://flow-like.com/download";
const RESULTS_ID = "registry-mine-results";
const EXPLORE_PACKAGES_HREF = exploreSearchHref({ type: "packages" });
const SKELETON_KEYS = Array.from(
	{ length: 8 },
	(_, index) => `registry-mine-skeleton-${index}`,
);
const SEARCH_OPTIONS = {
	fields: ["name", "metadata.name", "id", "description", "keywords"],
	boost: { name: 3, "metadata.name": 3, id: 2, keywords: 1.5 },
} as const;

/** Same key shape as the desktop Mine's `mineQueryKeys.maintained`, so publish/delete invalidations reach both. */
export const registryMineQueryKey = (user?: string) =>
	["mine-registry-maintained", user] as const;

const STATE_TONE: Record<RegistryMineState, HubTone> = {
	live: {
		text: "text-emerald-600 dark:text-emerald-400",
		dot: "bg-emerald-500",
	},
	in_review: { text: "text-sky-600 dark:text-sky-400", dot: "bg-sky-500" },
	rejected: { text: "text-destructive", dot: "bg-destructive" },
	disabled: { text: "text-destructive", dot: "bg-destructive" },
	deprecated: { text: "text-muted-foreground", dot: "bg-muted-foreground" },
};

/** Where "Manage" lands: review feedback and the restore card live in Releases. */
const MANAGE_TAB: Record<RegistryMineState, WorkspaceTab> = {
	live: "overview",
	in_review: "releases",
	rejected: "releases",
	disabled: "releases",
	deprecated: "overview",
};

function useRegistryMineLabels(): Record<RegistryMineFilter, string> {
	const { t } = useTranslation();
	return useMemo(
		() => ({
			all: t("all", "All"),
			live: t("live", "Live"),
			in_review: t("inReview", "In review"),
			rejected: t("rejected", "Rejected"),
			disabled: t("disabled", "Disabled"),
			deprecated: t("deprecated", "Deprecated"),
		}),
		[t],
	);
}

function useRegistryMine(auth: LibraryAuth) {
	const backend = useBackend();
	const backendReady = useBackendReady();
	const queryClient = useQueryClient();
	const pushedSignedIn = useAuthStatusStore((state) => state.signedIn);
	const signedIn = resolveLibrarySignedIn(pushedSignedIn, auth);
	const sub = auth?.user?.profile?.sub;

	const maintained = useQuery({
		queryKey: registryMineQueryKey(sub),
		queryFn: () =>
			backend.registryState.getOwnedPackages({
				access: "maintainer",
				includeDisabled: true,
				includeDeprecated: true,
				limit: MAINTAINER_LIMIT,
			}),
		enabled: backendReady && signedIn === true,
		staleTime: REGISTRY_STALE_MS,
	});

	const status: RegistryMineStatus =
		signedIn === undefined
			? "loading"
			: signedIn === false
				? "signed-out"
				: maintained.isLoadingError
					? "error"
					: maintained.isPending
						? "loading"
						: "ready";

	const packages = useMemo(
		() => (signedIn === true ? asArray(maintained.data?.packages) : []),
		[signedIn, maintained.data],
	);

	const refresh = useCallback(() => {
		void queryClient.invalidateQueries({
			queryKey: ["mine-registry-maintained"],
		});
	}, [queryClient]);

	return {
		status,
		packages,
		refetchFailed: signedIn === true && maintained.isRefetchError,
		refresh,
		retry: maintained.refetch,
		isRefreshing: maintained.isFetching,
		signedIn,
	};
}

function StatePill({ state }: { state: RegistryMineState }) {
	const labels = useRegistryMineLabels();
	return <HubStatePill label={labels[state]} tone={STATE_TONE[state]} />;
}

function MineFooter({
	pkg,
	state,
}: {
	pkg: PackageSummary;
	state: RegistryMineState;
}) {
	const { t } = useTranslation();
	const labels = useRegistryMineLabels();
	const rated = (pkg.ratingCount ?? 0) > 0;
	return (
		<PackageCardStats>
			<PackageCardStat
				value={formatCompact(pkg.downloadCount)}
				label={t("installs", "Installs")}
				valueClassName="tabular-nums"
			/>
			<PackageCardStat
				value={
					rated ? (
						<>
							<PackageRatingStar />
							{(pkg.avgRating ?? 0).toFixed(1)}
						</>
					) : (
						"—"
					)
				}
				label={t("rating", "Rating")}
				valueClassName="flex items-center gap-1 tabular-nums"
			/>
			<PackageCardStat
				value={labels[state]}
				label={t("state", "State")}
				valueClassName={cn("truncate", STATE_TONE[state].text)}
			/>
		</PackageCardStats>
	);
}

function useStatusLine(
	pkg: PackageSummary,
	state: RegistryMineState,
): { icon: LucideIcon; text: string; className: string } {
	const { t } = useTranslation();
	switch (state) {
		case "in_review":
			return {
				icon: History,
				text: t(
					"registryMineInReviewLine",
					"Waiting for review — goes live once approved",
				),
				className: STATE_TONE.in_review.text,
			};
		case "rejected":
			return {
				icon: AlertCircle,
				text: t(
					"registryMineRejectedLine",
					"Review rejected — see Releases for the feedback",
				),
				className: STATE_TONE.rejected.text,
			};
		case "disabled":
			return {
				icon: AlertCircle,
				text: t(
					"registryMineDisabledLine",
					"Hidden from the store — restore it in Releases",
				),
				className: STATE_TONE.disabled.text,
			};
		case "deprecated":
			return {
				icon: History,
				text: t(
					"registryMineDeprecatedLine",
					"Deprecated — no longer offered to new users",
				),
				className: "text-muted-foreground",
			};
		case "live":
			return {
				icon: ShieldCheck,
				text: isOwner(pkg.viewerPermission ?? 0)
					? t("registryMineOwnerLine", "You own this package")
					: t("registryMineMaintainerLine", "You maintain this package"),
				className: "text-muted-foreground",
			};
	}
}

function StatusLine({
	pkg,
	state,
}: {
	pkg: PackageSummary;
	state: RegistryMineState;
}) {
	const { icon: Icon, text, className } = useStatusLine(pkg, state);
	return (
		<p
			className={cn("flex h-5 min-w-0 items-center gap-1.5 text-xs", className)}
			title={text}
		>
			<Icon aria-hidden="true" className="size-3.5 shrink-0" />
			<span className="truncate">{text}</span>
		</p>
	);
}

function MenuLink({
	href,
	icon: Icon,
	children,
}: {
	href: string;
	icon: LucideIcon;
	children: ReactNode;
}) {
	return (
		<DropdownMenuItem asChild>
			<Link href={href}>
				<Icon className="text-muted-foreground" />
				{children}
			</Link>
		</DropdownMenuItem>
	);
}

function MoreMenu({ pkg }: { pkg: PackageSummary }) {
	const { t } = useTranslation();
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button
					variant="outline"
					size="icon"
					className="ml-auto size-8 shrink-0 text-muted-foreground"
					aria-label={t("moreActionsFor", "More actions for {{name}}", {
						name: pkg.name,
					})}
				>
					<MoreHorizontal />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className="w-56">
				<MenuLink
					href={packageStoreHref({ id: pkg.id, tab: "mine" })}
					icon={ExternalLink}
				>
					{t("viewStorePage", "View store page")}
				</MenuLink>
				<MenuLink
					href={packageWorkspaceHref({ id: pkg.id, tab: "access" })}
					icon={Users}
				>
					{t("accessAndPeople", "Access & people")}
				</MenuLink>
				<MenuLink
					href={packageWorkspaceHref({ id: pkg.id, tab: "releases" })}
					icon={History}
				>
					{t("workspaceTabReleases", "Releases")}
				</MenuLink>
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

function useVersionLabel(pkg: PackageSummary, state: RegistryMineState) {
	const { t } = useTranslation();
	if (state === "live")
		return t("versionLive", "v{{version}} live", {
			version: pkg.latestVersion,
		});
	if (state === "in_review")
		return t("versionInReview", "v{{version}} in review", {
			version: pkg.latestVersion,
		});
	return `v${pkg.latestVersion}`;
}

/** A stretched link opens the workspace from anywhere on the card; the title link stays the keyboard tab stop. */
function RegistryMineCard({ pkg }: { pkg: PackageSummary }) {
	const { t } = useTranslation();
	const state = registryMineState(pkg.status);
	const versionLabel = useVersionLabel(pkg, state);
	const overviewHref = packageWorkspaceHref({ id: pkg.id });
	const manageHref = packageWorkspaceHref({
		id: pkg.id,
		tab: MANAGE_TAB[state],
	});
	return (
		<div className="min-w-0" data-registry-mine-package={pkg.id}>
			<PackageCard
				pkg={pkg}
				href={null}
				titleHref={overviewHref}
				versionLabel={versionLabel}
				overlay={<StatePill state={state} />}
				className={cn(
					(state === "disabled" || state === "rejected") &&
						"border-dashed border-destructive/40",
				)}
				footer={
					<>
						<MineFooter pkg={pkg} state={state} />
						<Link
							href={overviewHref}
							tabIndex={-1}
							aria-hidden="true"
							data-registry-mine-card-link
							className="absolute inset-0 z-1 rounded-xl"
						/>
					</>
				}
				actions={
					<>
						<StatusLine pkg={pkg} state={state} />
						<div className="relative z-2 mt-2.5 flex min-w-0 items-center gap-1.5">
							<Button
								asChild
								size="sm"
								variant="secondary"
								className="h-8 min-w-0 flex-1"
							>
								<Link href={manageHref}>
									<span className="truncate">{t("manage", "Manage")}</span>
									<ArrowRight />
								</Link>
							</Button>
							<MoreMenu pkg={pkg} />
						</div>
					</>
				}
			/>
		</div>
	);
}

function StateFilters({
	counts,
	value,
	onChange,
}: {
	counts: Record<RegistryMineFilter, number>;
	value: RegistryMineFilter;
	onChange: (value: RegistryMineFilter) => void;
}) {
	const { t } = useTranslation();
	const labels = useRegistryMineLabels();
	const keys = registryMineFilterKeys(counts, value);
	if (keys.length <= 2 && value === "all") return null;
	return (
		<HubFilterChips
			label={t("filterByState", "Filter by state")}
			keys={keys}
			labels={labels}
			counts={counts}
			value={value}
			onChange={onChange}
			dot={(key) => (key === "all" ? null : STATE_TONE[key].dot)}
		/>
	);
}

function Toolbar({
	query,
	onQueryChange,
	sort,
	onSortChange,
}: {
	query: string;
	onQueryChange: (query: string) => void;
	sort: RegistryMineSort;
	onSortChange: (sort: RegistryMineSort) => void;
}) {
	const { t } = useTranslation();
	return (
		<div className="flex flex-col gap-3 sm:flex-row sm:items-center">
			<div className="min-w-0 flex-1">
				<HubSearchToolbar
					label={t("searchYourPackages", "Search your packages…")}
					controlsId={RESULTS_ID}
					query={query}
					onQueryChange={onQueryChange}
				/>
			</div>
			<Select
				value={sort}
				onValueChange={(value) => onSortChange(value as RegistryMineSort)}
			>
				<SelectTrigger
					aria-label={t("sortPackages", "Sort packages")}
					className="min-h-12 w-full shrink-0 gap-2 rounded-xl border-border/60 bg-background text-sm sm:w-44"
				>
					<SelectValue />
				</SelectTrigger>
				<SelectContent>
					<SelectItem value="attention">
						{t("needsAttention", "Needs attention")}
					</SelectItem>
					<SelectItem value="name">{t("sortByName", "Name")}</SelectItem>
				</SelectContent>
			</Select>
		</div>
	);
}

function EmptyMine() {
	const { t } = useTranslation();
	return (
		<div className="flex flex-col items-center gap-4" data-registry-mine-empty>
			<EmptyState
				icons={[Package, Code2, Sparkles]}
				title={t(
					"noPackagesInTheRegistryYet",
					"You have no packages in the registry yet",
				)}
				description={t(
					"localPackagesAreManagedInTheDesktopApp",
					"Local packages are built and managed in the Flow-Like desktop app. Publish one from there and it shows up here.",
				)}
				className="rounded-2xl border border-dashed border-border/30 bg-muted/5"
			/>
			<div className="flex flex-wrap items-center justify-center gap-2">
				<Button asChild>
					<a href={DESKTOP_DOWNLOAD_URL} target="_blank" rel="noreferrer">
						<Download />
						{t("getTheDesktopApp", "Get the desktop app")}
					</a>
				</Button>
				<Button asChild variant="outline">
					<Link href={EXPLORE_PACKAGES_HREF}>
						{t("explorePackages", "Explore packages")}
						<ArrowRight />
					</Link>
				</Button>
			</div>
		</div>
	);
}

/** Web Mine: registry packages the caller owns or maintains. Local checkouts live in the desktop Mine. */
export function RegistryMinePackages({
	auth,
	navigation,
}: RegistryMinePackagesProps) {
	const { t } = useTranslation();
	const mine = useRegistryMine(auth);
	const [query, setQuery] = useState("");
	const [filter, setFilter] = useState<RegistryMineFilter>("all");
	const [sort, setSort] = useState<RegistryMineSort>("attention");

	const counts = useMemo(
		() => registryMineCounts(mine.packages),
		[mine.packages],
	);
	const ordered = useMemo(
		() => sortRegistryMine(filterRegistryMine(mine.packages, filter), sort),
		[mine.packages, filter, sort],
	);
	const results = useSearch(ordered, query, SEARCH_OPTIONS);

	const { status } = mine;
	const hasList = status === "ready" && mine.packages.length > 0;

	return (
		<PackagesHubLayout
			subtitle={t(
				"registryMineSubtitle",
				"Packages you own or maintain in the registry.",
			)}
			actions={
				mine.signedIn === true ? (
					<Button
						variant="outline"
						disabled={mine.isRefreshing}
						onClick={mine.refresh}
					>
						<RefreshCw className={cn(mine.isRefreshing && "animate-spin")} />
						<span className="sr-only sm:not-sr-only">
							{t("refresh", "Refresh")}
						</span>
					</Button>
				) : null
			}
			navigation={navigation}
			toolbar={
				hasList ? (
					<Toolbar
						query={query}
						onQueryChange={setQuery}
						sort={sort}
						onSortChange={setSort}
					/>
				) : null
			}
			filters={
				hasList ? (
					<StateFilters counts={counts} value={filter} onChange={setFilter} />
				) : null
			}
		>
			<section
				id={RESULTS_ID}
				className="space-y-5"
				aria-busy={status === "loading"}
			>
				<output
					className="flex min-h-5 flex-wrap gap-4 text-sm text-muted-foreground"
					aria-live="polite"
				>
					{hasList &&
						t("countPackages", {
							defaultValue_one: "{{count}} package",
							defaultValue_other: "{{count}} packages",
							count: results.length,
						})}
					{status === "loading" && (
						<span className="sr-only">
							{t("store:loadingPackages", "Loading packages…")}
						</span>
					)}
				</output>
				{(status === "error" || mine.refetchFailed) && (
					<HubRetryAlert
						data-registry-mine-error
						message={t(
							"yourPackagesCouldNotBeLoaded",
							"Your packages could not be loaded from the registry.",
						)}
						busy={mine.isRefreshing}
						onRetry={mine.retry}
					/>
				)}
				{status === "loading" ? (
					<div className={PACKAGE_GRID_CLASS_NAME}>
						{SKELETON_KEYS.map((key) => (
							<PackageCardSkeleton key={key} />
						))}
					</div>
				) : status === "signed-out" ? (
					<HubSignInPrompt
						data-registry-mine-sign-in
						auth={auth}
						icon={Code2}
						title={t("signInToSeeYourPackages", "Sign in to see your packages")}
						description={t(
							"mineListsPackagesYouOwnOrMaintain",
							"Mine lists the packages you own or maintain in the registry.",
						)}
					/>
				) : status === "ready" && mine.packages.length === 0 ? (
					<EmptyMine />
				) : !hasList ? null : results.length === 0 ? (
					<HubNoMatches
						text={t(
							"noPackagesMatchThisFilterAndSearch",
							"No packages match this filter and search.",
						)}
						onShowAll={() => {
							setFilter("all");
							setQuery("");
						}}
					/>
				) : (
					<div className={PACKAGE_GRID_CLASS_NAME}>
						{results.map((pkg) => (
							<RegistryMineCard key={pkg.id} pkg={pkg} />
						))}
					</div>
				)}
			</section>
		</PackagesHubLayout>
	);
}
