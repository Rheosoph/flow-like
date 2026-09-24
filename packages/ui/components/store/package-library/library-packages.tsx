"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowRight,
	Download,
	ExternalLink,
	FileUp,
	Library,
	Loader2,
	LogIn,
	MoreHorizontal,
	Package,
	RefreshCw,
	ShieldAlert,
	Sparkles,
	Trash2,
} from "lucide-react";
import Link from "next/link";
import { type ReactNode, useMemo, useState } from "react";
import { toast } from "sonner";
import { useSearch } from "../../../hooks/use-search-index";
import { hasElevatedAccess } from "../../../lib/app-package-overview";
import { getErrorMessage } from "../../../lib/error-message";
import { usePackageCapabilities } from "../../../lib/package-capabilities";
import { readManifestWidgets } from "../../../lib/package-widgets";
import { cn } from "../../../lib/utils";
import { Alert, AlertDescription } from "../../ui/alert";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../../ui/alert-dialog";
import { Button } from "../../ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../../ui/dropdown-menu";
import { EmptyState } from "../../ui/empty-state";
import type { CompileStatus } from "../../ui/package-status-badge";
import {
	PackageCard,
	PackageCardStat,
	PackageCardStats,
	formatCompact,
} from "../package-card";
import {
	HubFilterChips,
	HubNoMatches,
	HubRetryAlert,
	HubSearchToolbar,
	HubSignInPrompt,
	HubStatePill,
	type HubTone,
	useHubSignIn,
} from "../packages-hub-controls";
import { PackagesHubLayout } from "../packages-hub-layout";
import {
	PACKAGE_GRID_CLASS_NAME,
	PackageCardSkeleton,
} from "../packages-store-page";
import {
	ClearWidgetPermissionsMenuItem,
	useMicroWidgetConsentEntries,
} from "../widget-permissions";
import {
	LIBRARY_ACCESS_FILTERS,
	LIBRARY_STATE_FILTERS,
	type LibraryAccess,
	type LibraryAccessFilter,
	type LibraryCounts,
	type LibraryEntry,
	type LibraryFilter,
	type LibraryState,
	type LibraryStateFilter,
	type LibraryUpdateCheck,
	filterLibrary,
	libraryStoreHref,
	librarySummary,
	libraryUpdateCheck,
} from "./library-model";
import {
	type LibraryAuth,
	type LibraryPackagesState,
	useLibraryPackages,
} from "./use-library-packages";

export interface LibraryMachine {
	getPackageStatus?: (packageId: string) => CompileStatus | undefined;
	onLoadLocal?: () => unknown;
}

export interface LibraryPackagesProps {
	auth: LibraryAuth;
	navigation?: ReactNode;
	/** Desktop only: installs, updates and compile status on this machine. */
	machine?: LibraryMachine;
}

const EXPLORE_PACKAGES_HREF = "/store/packages?tab=explore";
const RESULTS_ID = "library-package-results";
const SKELETON_KEYS = Array.from(
	{ length: 8 },
	(_, index) => `library-skeleton-${index}`,
);
const ALL_FILTER: LibraryFilter = { state: "all", access: "all" };
const SEARCH_OPTIONS = {
	fields: ["name", "id", "searchText"],
	boost: { name: 3, id: 2 },
} as const;
const KEEPS_ACCESS: ReadonlySet<LibraryAccess> = new Set([
	"bought",
	"shared",
	"owner",
	"maintainer",
]);

const STATE_TONE: Record<LibraryState, HubTone> = {
	not_installed: {
		text: "text-violet-600 dark:text-violet-400",
		dot: "bg-violet-500",
	},
	ready: {
		text: "text-emerald-600 dark:text-emerald-400",
		dot: "bg-emerald-500",
	},
	update: { text: "text-primary", dot: "bg-primary" },
	problem: { text: "text-destructive", dot: "bg-destructive" },
};

function useLibraryLabels() {
	const { t } = useTranslation();
	return useMemo(
		() => ({
			state: {
				all: t("all", "All"),
				not_installed: t("notOnThisMachine", "Not on this machine"),
				ready: t("ready", "Ready"),
				update: t("libraryUpdateAvailable", "Update available"),
				problem: t("libraryProblem", "Problem"),
			} satisfies Record<LibraryStateFilter, string>,
			access: {
				all: t("allAccess", "Any access"),
				bought: t("libraryAccessBought", "Bought"),
				shared: t("libraryAccessShared", "Shared with you"),
				owner: t("store:owner", "Owner"),
				maintainer: t("store:maintainer", "Maintainer"),
				free: t("free", "Free"),
				local: t("local", "Local"),
				unknown: "—",
			} satisfies Record<LibraryAccessFilter | LibraryAccess, string>,
		}),
		[t],
	);
}

/** Runs the host's file-dialog flow, reports its failure, and refreshes the lists either way. */
function useLoadLocal(library: LibraryPackagesState, machine?: LibraryMachine) {
	const { t } = useTranslation();
	const [loading, setLoading] = useState(false);
	const onLoadLocal = machine?.onLoadLocal;
	if (!onLoadLocal) return { load: undefined, loading };

	const load = async () => {
		setLoading(true);
		try {
			await onLoadLocal();
		} catch (error) {
			toast.error(
				t("failedToLoadLocalPackage", "Could not load the .wasm: {{message}}", {
					message: getErrorMessage(error),
				}),
			);
		} finally {
			setLoading(false);
			library.refresh();
		}
	};
	return { load, loading };
}

function HeaderActions({
	library,
	machine,
}: {
	library: LibraryPackagesState;
	machine?: LibraryMachine;
}) {
	const { t } = useTranslation();
	const local = useLoadLocal(library, machine);

	return (
		<div className="flex items-center gap-2">
			<Button
				variant="outline"
				disabled={library.isRefreshing}
				onClick={library.refresh}
			>
				<RefreshCw className={cn(library.isRefreshing && "animate-spin")} />
				<span className="sr-only sm:not-sr-only">
					{machine
						? t("checkForUpdates", "Check for updates")
						: t("refresh", "Refresh")}
				</span>
			</Button>
			{local.load && (
				<Button variant="outline" disabled={local.loading} onClick={local.load}>
					{local.loading ? <Loader2 className="animate-spin" /> : <FileUp />}
					<span className="sr-only sm:not-sr-only">
						{t("loadLocalWasm", "Load local .wasm")}
					</span>
				</Button>
			)}
		</div>
	);
}

function Filters({
	counts,
	filter,
	onChange,
	machine,
}: {
	counts: LibraryCounts;
	filter: LibraryFilter;
	onChange: (filter: LibraryFilter) => void;
	machine: boolean;
}) {
	const { t } = useTranslation();
	const labels = useLibraryLabels();
	const accessKeys = LIBRARY_ACCESS_FILTERS.filter(
		(key) => key === "all" || counts.access[key] > 0 || filter.access === key,
	);
	/** An active access filter stays visible after its last entry leaves, so it can be cleared. */
	const showAccess = accessKeys.length > 2 || filter.access !== "all";
	return (
		<div className="flex min-w-0 flex-wrap items-center gap-3">
			{machine && (
				<HubFilterChips
					label={t("filterByState", "Filter by state")}
					keys={LIBRARY_STATE_FILTERS}
					labels={labels.state}
					counts={counts.state}
					value={filter.state}
					onChange={(state) => onChange({ ...filter, state })}
					dot={(key) => (key === "all" ? null : STATE_TONE[key].dot)}
				/>
			)}
			{showAccess && (
				<HubFilterChips
					label={t("filterByAccess", "Filter by access")}
					keys={accessKeys}
					labels={labels.access}
					counts={counts.access}
					value={filter.access}
					onChange={(access) => onChange({ ...filter, access })}
				/>
			)}
		</div>
	);
}

function StatePill({ state }: { state: LibraryState }) {
	const labels = useLibraryLabels();
	return <HubStatePill label={labels.state[state]} tone={STATE_TONE[state]} />;
}

function useMachineStat(
	entry: LibraryEntry,
	status: CompileStatus | undefined,
): { label: string; className: string } {
	const { t } = useTranslation();
	if (status === "error")
		return { label: t("failed", "Failed"), className: "text-destructive" };
	if (status === "downloading" || status === "compiling")
		return {
			label: t("compiling", "Compiling"),
			className: "text-sky-600 dark:text-sky-400",
		};
	if (entry.installed)
		return { label: t("ready", "Ready"), className: STATE_TONE.ready.text };
	return { label: t("no", "No"), className: "text-muted-foreground" };
}

function LibraryFooter({
	entry,
	machine,
	status,
}: {
	entry: LibraryEntry;
	machine: boolean;
	status: CompileStatus | undefined;
}) {
	const { t } = useTranslation();
	const labels = useLibraryLabels();
	const stat = useMachineStat(entry, status);
	const accessStat = (
		<PackageCardStat
			value={labels.access[entry.access]}
			label={t("access", "Access")}
			valueClassName={cn(
				"truncate",
				entry.access === "bought" && "text-primary",
				entry.access === "unknown" && "text-muted-foreground",
			)}
		/>
	);
	if (!machine) {
		const summary = librarySummary(entry);
		return (
			<PackageCardStats>
				{accessStat}
				<PackageCardStat
					value={`v${summary.latestVersion}`}
					label={t("version", "Version")}
					valueClassName="truncate"
				/>
				<PackageCardStat
					value={formatCompact(summary.downloadCount)}
					label={t("installs", "Installs")}
					valueClassName="tabular-nums"
				/>
			</PackageCardStats>
		);
	}
	return (
		<PackageCardStats>
			{accessStat}
			<PackageCardStat
				value={entry.installed ? `v${entry.installed.version}` : "—"}
				label={t("installed", "Installed")}
				valueClassName="truncate"
			/>
			<PackageCardStat
				value={stat.label}
				label={t("onThisMachine", "On this machine")}
				valueClassName={cn("truncate", stat.className)}
			/>
		</PackageCardStats>
	);
}

function MachineCaption({
	entry,
	check,
}: {
	entry: LibraryEntry;
	check: LibraryUpdateCheck;
}) {
	const { t } = useTranslation();
	const version = entry.update?.latestVersion;
	const text = (() => {
		switch (entry.state) {
			case "problem":
				return entry.access === "local"
					? t(
							"libraryCaptionLocalProblem",
							"This local .wasm failed on this machine — load it again",
						)
					: t(
							"libraryCaptionProblem",
							"Install failed on this machine — try again",
						);
			case "not_installed":
				return t(
					"libraryCaptionNotInstalled",
					"Yours to install — not on this machine yet",
				);
			case "update":
				if (check.status === "verified" && check.newTags.length > 0) {
					return t("libraryCaptionUpdateAdds", "v{{version}} adds {{tags}}", {
						version,
						tags: check.newTags.join(", "),
					});
				}
				return check.status === "unverified"
					? t(
							"libraryCaptionUpdateUnverified",
							"v{{version}} is available — its permissions could not be checked",
							{ version },
						)
					: t("libraryCaptionUpdate", "v{{version}} is available", {
							version,
						});
			case "ready":
				return entry.access === "local"
					? t("libraryCaptionLocal", "Loaded from a local .wasm file")
					: t("libraryCaptionReady", "Ready in your catalog");
		}
	})();
	return (
		<p
			className={cn(
				"mb-2 flex h-5 min-w-0 items-center truncate text-xs",
				entry.state === "ready"
					? "text-muted-foreground"
					: STATE_TONE[entry.state].text,
			)}
			title={text}
		>
			{text}
		</p>
	);
}

function needsReview(check: LibraryUpdateCheck): boolean {
	return (
		check.status === "unverified" ||
		(check.status === "verified" && check.needsConsent)
	);
}

function UpdateAction({
	entry,
	check,
	busy,
	className,
	onUpdate,
}: {
	entry: LibraryEntry;
	check: LibraryUpdateCheck;
	busy: boolean;
	className: string;
	onUpdate: () => void;
}) {
	const { t } = useTranslation();
	const checking = check.status === "checking";
	const review = needsReview(check);
	const label = checking
		? t("checkingPermissions", "Checking permissions…")
		: check.status === "unverified"
			? t("reviewUpdate", "Review update")
			: check.status === "verified" && check.needsConsent
				? t("reviewCountNewPermissions", {
						defaultValue_one: "Review {{count}} new permission",
						defaultValue_other: "Review {{count}} new permissions",
						count: check.newTags.length,
					})
				: t("updateToVersion", "Update to v{{version}}", {
						version: entry.update?.latestVersion,
					});
	return (
		<Button
			size="sm"
			variant={review ? "default" : "secondary"}
			className={className}
			disabled={busy || checking}
			onClick={onUpdate}
		>
			{busy || checking ? (
				<Loader2 className="animate-spin" />
			) : review ? (
				<ShieldAlert />
			) : (
				<RefreshCw />
			)}
			<span className="truncate">{label}</span>
		</Button>
	);
}

function PrimaryAction({
	entry,
	machine,
	busy,
	check,
	local,
	onInstall,
	onUpdate,
}: {
	entry: LibraryEntry;
	machine: boolean;
	busy: boolean;
	check: LibraryUpdateCheck;
	local: ReturnType<typeof useLoadLocal>;
	onInstall: () => void;
	onUpdate: () => void;
}) {
	const { t } = useTranslation();
	const className = "h-8 min-w-0 flex-1";
	if (entry.access === "local") {
		if (!machine || entry.state !== "problem" || !local.load) return null;
		return (
			<Button
				size="sm"
				variant="secondary"
				className={className}
				disabled={busy || local.loading}
				onClick={local.load}
			>
				{local.loading ? <Loader2 className="animate-spin" /> : <FileUp />}
				<span className="truncate">
					{t("loadLocalWasm", "Load local .wasm")}
				</span>
			</Button>
		);
	}
	if (!machine || entry.state === "ready") {
		return (
			<Button asChild size="sm" variant="secondary" className={className}>
				<Link href={libraryStoreHref(entry.id)}>
					<span className="truncate">
						{t("viewStorePage", "View store page")}
					</span>
					<ArrowRight />
				</Link>
			</Button>
		);
	}
	if (entry.state === "update" && entry.update) {
		return (
			<UpdateAction
				entry={entry}
				check={check}
				busy={busy}
				className={className}
				onUpdate={onUpdate}
			/>
		);
	}
	return (
		<Button
			size="sm"
			variant="secondary"
			className={className}
			disabled={busy}
			onClick={onInstall}
		>
			{busy ? (
				<Loader2 className="animate-spin" />
			) : entry.state === "problem" ? (
				<RefreshCw className="text-destructive" />
			) : (
				<Download />
			)}
			<span className="truncate">
				{entry.state === "problem"
					? t("retry", "Retry")
					: t("install", "Install")}
			</span>
		</Button>
	);
}

function MoreMenuItems({
	entry,
	machine,
	busy,
	onInstall,
	onUninstall,
}: {
	entry: LibraryEntry;
	machine: boolean;
	busy: boolean;
	onInstall: () => void;
	onUninstall: () => void;
}) {
	const { t } = useTranslation();
	const grants = useMicroWidgetConsentEntries({ packageId: entry.id });
	const hasWidgets =
		readManifestWidgets(entry.installed?.manifest).length > 0 ||
		grants.length > 0;
	return (
		<>
			{machine && !entry.installed && (
				<DropdownMenuItem disabled={busy} onSelect={onInstall}>
					<Download />
					{t("installOnThisMachine", "Install on this machine")}
				</DropdownMenuItem>
			)}
			{entry.access !== "local" && (
				<DropdownMenuItem asChild>
					<Link href={libraryStoreHref(entry.id)}>
						<ExternalLink />
						{t("viewStorePage", "View store page")}
					</Link>
				</DropdownMenuItem>
			)}
			{hasWidgets && (
				<ClearWidgetPermissionsMenuItem
					packageId={entry.id}
					packageName={entry.name}
				/>
			)}
			{machine && entry.installed && (
				<>
					<DropdownMenuSeparator />
					<DropdownMenuItem
						variant="destructive"
						className="items-start"
						disabled={busy}
						onSelect={onUninstall}
					>
						<Trash2 className="mt-0.5" />
						<span className="flex flex-col">
							<span>{t("uninstall", "Uninstall")}</span>
							{KEEPS_ACCESS.has(entry.access) && (
								<span className="text-xs text-muted-foreground">
									{t(
										"youKeepAccessReinstallAnytime",
										"You keep access — reinstall anytime",
									)}
								</span>
							)}
						</span>
					</DropdownMenuItem>
				</>
			)}
		</>
	);
}

function MoreMenu(props: Parameters<typeof MoreMenuItems>[0]) {
	const { t } = useTranslation();
	return (
		<DropdownMenu>
			<DropdownMenuTrigger asChild>
				<Button
					variant="outline"
					size="icon"
					className="ml-auto size-8 shrink-0 text-muted-foreground"
					aria-label={t("moreActionsFor", "More actions for {{name}}", {
						name: props.entry.name,
					})}
				>
					<MoreHorizontal />
				</Button>
			</DropdownMenuTrigger>
			<DropdownMenuContent align="end" className="w-64">
				<MoreMenuItems {...props} />
			</DropdownMenuContent>
		</DropdownMenu>
	);
}

function NewPermissionList({ tags }: { tags: readonly string[] }) {
	const capabilities = usePackageCapabilities(tags);
	return (
		<ul className="space-y-1.5" data-update-consent-tags>
			{capabilities.map((capability) => (
				<li key={capability.key} className="flex items-center gap-2 text-sm">
					<span
						className={cn(
							"rounded border px-1.5 py-1 font-mono text-[10px] leading-none",
							hasElevatedAccess([capability.key])
								? "border-primary/35 bg-primary/10 text-primary"
								: "border-border/60 bg-muted/40 text-muted-foreground",
						)}
					>
						{capability.key}
					</span>
					<span>{capability.label}</span>
				</li>
			))}
		</ul>
	);
}

function UpdateReviewDialog({
	entry,
	check,
	open,
	onOpenChange,
	onConfirm,
}: {
	entry: LibraryEntry;
	check: LibraryUpdateCheck;
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onConfirm: () => void;
}) {
	const { t } = useTranslation();
	const values = { name: entry.name, version: entry.update?.latestVersion };
	const unverified = check.status === "unverified";
	return (
		<AlertDialog open={open} onOpenChange={onOpenChange}>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>
						{unverified
							? t(
									"updateWithoutPermissionCheckTitle",
									"Update without a permission check?",
								)
							: t("allowNewPermissionsTitle", "Allow new permissions?")}
					</AlertDialogTitle>
					<AlertDialogDescription
						data-update-unverified={unverified || undefined}
					>
						{unverified
							? t(
									"updatePermissionsUnverifiedDescription",
									"The registry could not tell which permissions {{name}} v{{version}} asks for, so it may ask for more than the version on this machine. Check its store page first.",
									values,
								)
							: t(
									"allowNewPermissionsDescription",
									"{{name}} v{{version}} asks for more than the version on this machine:",
									values,
								)}
					</AlertDialogDescription>
				</AlertDialogHeader>
				{check.status === "verified" && (
					<NewPermissionList tags={check.newTags} />
				)}
				<AlertDialogFooter>
					<AlertDialogCancel>{t("cancel", "Cancel")}</AlertDialogCancel>
					{unverified && (
						<Button asChild variant="outline">
							<Link href={libraryStoreHref(entry.id)}>
								{t("viewStorePage", "View store page")}
							</Link>
						</Button>
					)}
					<AlertDialogAction onClick={onConfirm}>
						{unverified
							? t("updateAnyway", "Update anyway")
							: t("allowAndUpdate", "Allow and update")}
					</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

function LibraryPackageCard({
	entry,
	library,
	machine,
}: {
	entry: LibraryEntry;
	library: LibraryPackagesState;
	machine?: LibraryMachine;
}) {
	const [reviewOpen, setReviewOpen] = useState(false);
	const local = useLoadLocal(library, machine);
	const onMachine = Boolean(machine);
	const status = machine?.getPackageStatus?.(entry.id);
	const busy =
		library.pending.has(entry.id) ||
		status === "downloading" ||
		status === "compiling";
	const summary = useMemo(() => librarySummary(entry), [entry]);
	const check = useMemo(
		() => libraryUpdateCheck(entry, library.summariesPending),
		[entry, library.summariesPending],
	);
	const review = needsReview(check);
	const installed = entry.installed;
	const versionLabel = installed
		? entry.update
			? `v${installed.version} → v${entry.update.latestVersion}`
			: `v${installed.version}`
		: undefined;

	const install = () =>
		library.install({ id: entry.id, version: installed?.version });
	const runUpdate = () => {
		if (entry.update) {
			library.update({ id: entry.id, version: entry.update.latestVersion });
		}
	};

	return (
		<div className="min-w-0" data-library-package={entry.id}>
			<PackageCard
				pkg={summary}
				href={null}
				titleHref={
					entry.access === "local" ? undefined : libraryStoreHref(entry.id)
				}
				versionLabel={versionLabel}
				overlay={onMachine ? <StatePill state={entry.state} /> : undefined}
				className={cn(
					onMachine &&
						entry.state === "not_installed" &&
						"border-dashed border-violet-500/50",
				)}
				footer={
					<LibraryFooter entry={entry} machine={onMachine} status={status} />
				}
				actions={
					<>
						{onMachine && <MachineCaption entry={entry} check={check} />}
						<div className="flex min-w-0 items-center gap-1.5">
							<PrimaryAction
								entry={entry}
								machine={onMachine}
								busy={busy}
								check={check}
								local={local}
								onInstall={install}
								onUpdate={() => (review ? setReviewOpen(true) : runUpdate())}
							/>
							<MoreMenu
								entry={entry}
								machine={onMachine}
								busy={busy}
								onInstall={install}
								onUninstall={() => library.uninstall(entry.id)}
							/>
						</div>
					</>
				}
			/>
			{review && (
				<UpdateReviewDialog
					entry={entry}
					check={check}
					open={reviewOpen}
					onOpenChange={setReviewOpen}
					onConfirm={runUpdate}
				/>
			)}
		</div>
	);
}

function SignedOutBanner({ auth }: { auth: LibraryAuth }) {
	const { t } = useTranslation();
	const signIn = useHubSignIn(auth);
	return (
		<Alert className="rounded-xl" data-library-signed-out>
			<LogIn className="h-4 w-4" />
			<AlertDescription className="flex flex-wrap items-center justify-between gap-3">
				{t(
					"signInToSeeBoughtAndSharedPackages",
					"Sign in to see bought and shared packages.",
				)}
				<Button variant="outline" size="sm" onClick={signIn}>
					{t("signIn", "Sign in")}
				</Button>
			</AlertDescription>
		</Alert>
	);
}

function EmptyLibrary() {
	const { t } = useTranslation();
	return (
		<div className="flex flex-col items-center gap-4" data-library-empty>
			<EmptyState
				icons={[Download, Package, Sparkles]}
				title={t("nothingInYourLibraryYet", "Nothing in your library yet")}
				description={t(
					"packagesYouBuyInstallOrGetSharedShowUpHere",
					"Packages you buy, install or get shared show up here.",
				)}
				className="rounded-2xl border border-dashed border-border/30 bg-muted/5"
			/>
			<Button asChild variant="outline">
				<Link href={EXPLORE_PACKAGES_HREF}>
					{t("explorePackages", "Explore packages")}
					<ArrowRight />
				</Link>
			</Button>
		</div>
	);
}

export function LibraryPackages({
	auth,
	navigation,
	machine,
}: LibraryPackagesProps) {
	const { t } = useTranslation();
	const getPackageStatus = machine?.getPackageStatus;
	const library = useLibraryPackages({ auth, statusOf: getPackageStatus });
	const [query, setQuery] = useState("");
	const [filter, setFilter] = useState<LibraryFilter>(ALL_FILTER);

	const { entries, counts, registryStatus } = library;
	const filtered = useMemo(
		() => filterLibrary(entries, filter),
		[entries, filter],
	);
	const results = useSearch(filtered, query, SEARCH_OPTIONS);

	const signedOut = registryStatus === "signed-out";
	const isLoading =
		library.isLoading || (entries.length === 0 && registryStatus === "loading");
	const showSignIn = !isLoading && signedOut && entries.length === 0;
	const isEmpty =
		!isLoading &&
		!signedOut &&
		registryStatus !== "error" &&
		!library.installedError &&
		entries.length === 0;
	const hasList = !isLoading && entries.length > 0;

	return (
		<PackagesHubLayout
			subtitle={t(
				"packagesYouCanUseBoughtSharedWithYouOrFree",
				"Packages you can use — bought, shared with you, or free.",
			)}
			actions={<HeaderActions library={library} machine={machine} />}
			navigation={navigation}
			toolbar={
				hasList ? (
					<HubSearchToolbar
						label={t("searchYourLibrary", "Search your library…")}
						controlsId={RESULTS_ID}
						query={query}
						onQueryChange={setQuery}
					/>
				) : null
			}
			filters={
				hasList ? (
					<Filters
						counts={counts}
						filter={filter}
						onChange={setFilter}
						machine={Boolean(machine)}
					/>
				) : null
			}
		>
			<section id={RESULTS_ID} className="space-y-5" aria-busy={isLoading}>
				<output
					className="flex min-h-5 flex-wrap gap-4 text-sm text-muted-foreground"
					aria-live="polite"
				>
					{hasList && (
						<span>
							{t("countPackages", {
								defaultValue_one: "{{count}} package",
								defaultValue_other: "{{count}} packages",
								count: results.length,
							})}
						</span>
					)}
					{hasList && registryStatus === "loading" && (
						<span className="flex items-center gap-1.5">
							<Loader2
								aria-hidden="true"
								className="h-3.5 w-3.5 animate-spin"
							/>
							{t("checkingTheRegistry", "Checking the registry…")}
						</span>
					)}
				</output>
				{hasList && signedOut && <SignedOutBanner auth={auth} />}
				{registryStatus === "error" && (
					<HubRetryAlert
						data-library-error
						message={t(
							"yourLibraryCouldNotBeLoaded",
							"Your library could not be loaded from the registry.",
						)}
						busy={library.isRefreshing}
						onRetry={library.retryRegistry}
					/>
				)}
				{Boolean(library.installedError) && (
					<HubRetryAlert
						data-library-error
						message={t(
							"installedPackagesCouldNotBeLoaded",
							"Installed packages could not be loaded.",
						)}
						busy={library.isRefreshing}
						onRetry={library.retryInstalled}
					/>
				)}
				{isLoading ? (
					<div className={PACKAGE_GRID_CLASS_NAME}>
						{SKELETON_KEYS.map((key) => (
							<PackageCardSkeleton key={key} />
						))}
					</div>
				) : showSignIn ? (
					<HubSignInPrompt
						data-library-sign-in
						auth={auth}
						icon={Library}
						title={t("signInToSeeYourLibrary", "Sign in to see your library")}
						description={t(
							"libraryListsPackagesYouBoughtOrThatWereSharedWithYou",
							"Your library lists the packages you bought or that were shared with you.",
						)}
					/>
				) : isEmpty ? (
					<EmptyLibrary />
				) : !hasList ? null : results.length === 0 ? (
					<HubNoMatches
						text={t(
							"nothingInYourLibraryMatchesTheseFilters",
							"Nothing in your library matches these filters.",
						)}
						onShowAll={() => {
							setFilter(ALL_FILTER);
							setQuery("");
						}}
					/>
				) : (
					<div className={PACKAGE_GRID_CLASS_NAME}>
						{results.map((entry) => (
							<LibraryPackageCard
								key={entry.id}
								entry={entry}
								library={library}
								machine={machine}
							/>
						))}
					</div>
				)}
			</section>
		</PackagesHubLayout>
	);
}
