"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import {
	AlertTriangle,
	ArrowRight,
	ChevronLeft,
	ChevronRight,
	CloudOff,
	ExternalLink,
	FileCode,
	LogIn,
	Package,
	RotateCw,
} from "lucide-react";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import {
	type ReactNode,
	Suspense,
	useCallback,
	useEffect,
	useMemo,
	useState,
} from "react";
import { useAssetImage } from "../../../hooks/use-asset-image";
import {
	hashToGradient,
	useThemeInfo,
} from "../../../hooks/use-theme-gradient";
import { getErrorMessage } from "../../../lib/error-message";
import { asArray } from "../../../lib/response-shape";
import type { RegistryEntry } from "../../../lib/schema/wasm";
import { cn } from "../../../lib/utils";
import type { GenericFetcher } from "../../pages/store/store-package-detail";
import {
	Button,
	EmptyState,
	Skeleton,
	Tabs,
	TabsContent,
	TabsList,
	TabsTrigger,
} from "../../ui";
import { getPackageInitials } from "../package-card";
import { packageStoreHref } from "../package-navigation";
import { AccessTab } from "./access-tab";
import { ListingTab } from "./listing-tab";
import { NodesTab } from "./nodes-tab";
import { OverviewTab } from "./overview-tab";
import { ReleasesTab } from "./releases-tab";
import {
	type RegistryPackageAuth,
	useRegistryPackage,
} from "./use-registry-package";
import { invalidatePackageLists, usePackageMeta } from "./use-workspace-data";
import {
	type WorkspaceTab,
	isWorkspaceTab,
	workspaceTabFromParam,
} from "./workspace-href";
import {
	type WorkspaceAccess,
	type WorkspaceAuthState,
	type WorkspaceBanner,
	listingHealth,
	liveVersion,
	registryState,
	workspaceAccess,
	workspaceTabs,
} from "./workspace-model";
import {
	CountBadge,
	RegistryStatePill,
	StatePill,
	VisibilityLabel,
	WorkspaceNotice,
} from "./workspace-parts";

export interface PackageWorkspaceLocalHeader {
	name?: string;
	id?: string;
	iconSrc?: string;
	/** Version in the linked checkout's manifest. */
	version?: string;
	/** Extra chips after the id, e.g. the source language. */
	badges?: ReactNode;
	/** Replaces the registry-derived state pill. */
	state?: ReactNode;
	/** Secondary header actions, left of the primary action. */
	actions?: ReactNode;
}

/** Present only when a checkout is linked; the gate treats it as "has local". */
export interface PackageWorkspaceLocal {
	header?: PackageWorkspaceLocalHeader;
	primaryAction?: ReactNode;
	overview?: { main?: ReactNode; aside?: ReactNode };
	nodes?: ReactNode;
	nodeCount?: number;
	test?: ReactNode;
	manifest?: ReactNode;
}

/** Host capabilities that exist without a linked checkout. */
export interface PackageWorkspaceDesktop {
	/** Shown in Test and Manifest when no checkout is linked. */
	linkFolder?: ReactNode;
	/** Offer "Install for testing" on Releases. */
	canInstall?: boolean;
}

export interface PackageWorkspaceProps {
	packageId?: string;
	fetcher: GenericFetcher;
	auth?: RegistryPackageAuth;
	local?: PackageWorkspaceLocal;
	/** True while a requested checkout is still resolving; the gate waits instead of deciding without it. */
	localPending?: boolean;
	desktop?: PackageWorkspaceDesktop;
}

type ShellAccess = Extract<
	WorkspaceAccess,
	{ mode: "full" | "local" | "local_only" }
>;

const MINE_HREF = "/store/packages?tab=mine";

export function PackageWorkspace(props: Readonly<PackageWorkspaceProps>) {
	return (
		<Suspense fallback={<PackageWorkspaceSkeleton />}>
			<WorkspaceGate {...props} />
		</Suspense>
	);
}

function WorkspaceGate({
	packageId,
	fetcher,
	auth,
	local,
	localPending,
	desktop,
}: Readonly<PackageWorkspaceProps>) {
	const router = useRouter();
	const signIn = useWorkspaceSignIn(auth);
	const registry = useRegistryPackage(packageId, fetcher, auth, {
		localFallback: false,
	});
	const authState = useSettledAuthState(registry.authState);
	const entry = registry.source === "registry" ? registry.entry : undefined;
	const hasLocal = !!local;
	const permission = entry?.currentUserPermission;

	const access = useMemo(
		() =>
			workspaceAccess({
				packageId,
				hasLocal,
				localPending,
				auth: authState,
				remote: {
					status: registry.status,
					source: registry.source,
					permission,
				},
			}),
		[
			packageId,
			hasLocal,
			localPending,
			authState,
			registry.status,
			registry.source,
			permission,
		],
	);

	const redirectHref = access.mode === "redirect" ? access.href : undefined;
	useEffect(() => {
		if (redirectHref) router.replace(redirectHref);
	}, [redirectHref, router]);

	switch (access.mode) {
		case "loading":
		case "redirect":
			return <PackageWorkspaceSkeleton />;
		case "not_found":
			return <WorkspaceUnavailable kind="not_found" packageId={packageId} />;
		case "session_expired":
			return (
				<WorkspaceUnavailable
					kind="session_expired"
					packageId={packageId}
					onSignIn={signIn}
				/>
			);
		case "error":
			return (
				<WorkspaceUnavailable
					kind="error"
					packageId={packageId}
					error={registry.error}
					retrying={registry.isFetching}
					onRetry={registry.retry}
				/>
			);
		default:
			return (
				<WorkspaceShell
					access={access}
					sessionExpired={authState === "expired"}
					entry={entry}
					packageId={packageId}
					fetcher={fetcher}
					auth={auth}
					local={local}
					desktop={desktop}
					onRetry={registry.retry}
					onSignIn={signIn}
				/>
			);
	}
}

/**
 * Hosts start renewing a stored expired user in an effect right after it
 * loads, so `expired` only counts once it has survived a commit; otherwise
 * every returning maintainer would see the sign-in page flash.
 */
function useSettledAuthState(state: WorkspaceAuthState): WorkspaceAuthState {
	const [committed, setCommitted] = useState(state);
	useEffect(() => setCommitted(state), [state]);
	return state === "expired" && committed !== "expired" ? "unknown" : state;
}

/** Sends the viewer through sign-in and back to this workspace. */
function useWorkspaceSignIn(auth: RegistryPackageAuth) {
	return useMemo(() => {
		if (!auth?.signinRedirect) return undefined;
		return () => {
			void auth.signinRedirect?.({
				url_state:
					typeof window === "undefined"
						? undefined
						: window.location.pathname + window.location.search,
			});
		};
	}, [auth]);
}

function WorkspaceFrame({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<main className="flex min-h-0 min-w-0 w-full flex-1 flex-col overflow-hidden">
			<div className="min-h-0 flex-1 overflow-auto [scrollbar-gutter:stable]">
				<div className="mx-auto w-full max-w-350 px-4 pt-5 pb-12 sm:px-8">
					{children}
				</div>
			</div>
		</main>
	);
}

function WorkspaceBreadcrumb({ label }: Readonly<{ label?: string }>) {
	const { t } = useTranslation("common");
	return (
		<nav
			aria-label={t("workspaceBreadcrumb", "Breadcrumb")}
			className="flex h-5 min-w-0 items-center gap-1.5 text-[13px] text-muted-foreground"
		>
			<Link
				href={MINE_HREF}
				className="inline-flex shrink-0 items-center gap-1 rounded-sm outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
			>
				<ChevronLeft className="size-3.5" />
				{t("packages", "Packages")}
			</Link>
			{label && (
				<>
					<ChevronRight className="size-3.5 shrink-0" />
					<span className="truncate font-mono text-xs text-foreground">
						{label}
					</span>
				</>
			)}
		</nav>
	);
}

export function PackageWorkspaceSkeleton() {
	return (
		<WorkspaceFrame>
			<Skeleton className="h-4 w-40" />
			<div className="mt-4 flex items-center gap-4">
				<Skeleton className="size-14 rounded-xl sm:size-16" />
				<div className="min-w-0 flex-1 space-y-2">
					<Skeleton className="h-7 w-64 max-w-full" />
					<Skeleton className="h-4 w-80 max-w-full" />
				</div>
				<Skeleton className="hidden h-9 w-32 sm:block" />
			</div>
			<Skeleton className="mt-6 h-10 w-full" />
			<div className="mt-5 grid gap-5 lg:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
				<Skeleton className="h-72 w-full rounded-xl" />
				<div className="space-y-4">
					<Skeleton className="h-44 w-full rounded-xl" />
					<Skeleton className="h-28 w-full rounded-xl" />
				</div>
			</div>
		</WorkspaceFrame>
	);
}

function WorkspaceUnavailable({
	kind,
	packageId,
	error,
	retrying,
	onRetry,
	onSignIn,
}: Readonly<{
	kind: "not_found" | "error" | "session_expired";
	packageId?: string;
	error?: unknown;
	retrying?: boolean;
	onRetry?: () => void;
	onSignIn?: () => void;
}>) {
	const { t } = useTranslation("common");
	const router = useRouter();
	const back = {
		label: t("backToPackages", "Back to packages"),
		onClick: () => router.push(MINE_HREF),
	};
	const primary =
		kind === "error" && onRetry
			? {
					label: retrying
						? t("workspaceRetrying", "Retrying…")
						: t("retry", "Retry"),
					onClick: onRetry,
				}
			: kind === "session_expired" && onSignIn
				? { label: t("signIn", "Sign in"), onClick: onSignIn }
				: undefined;
	const copy = {
		not_found: {
			title: t("workspaceNotFoundTitle", "Package not found"),
			description: t(
				"workspaceNotFoundDescription",
				"There is no package with this id that you can manage here.",
			),
		},
		error: {
			title: t("workspaceLoadFailedTitle", "Couldn't load this package"),
			description: t(
				"workspaceLoadFailedDescription",
				"{{message}} — check your connection and try again.",
				{
					message: error
						? getErrorMessage(error)
						: t("workspaceRegistryUnreachable", "The registry didn't answer"),
				},
			),
		},
		session_expired: {
			title: t("workspaceSessionExpiredTitle", "Your session expired"),
			description: t(
				"workspaceSessionExpiredPageDescription",
				"Sign in again to manage this package.",
			),
		},
	}[kind];

	return (
		<WorkspaceFrame>
			<WorkspaceBreadcrumb label={packageId} />
			<div className="mt-10 flex justify-center">
				<EmptyState
					icons={[kind === "session_expired" ? LogIn : Package]}
					title={copy.title}
					description={copy.description}
					action={primary ? [primary, back] : [back]}
				/>
			</div>
		</WorkspaceFrame>
	);
}

function WorkspaceIcon({
	id,
	name,
	iconUrl,
	iconSrc,
}: Readonly<{ id: string; name: string; iconUrl?: string; iconSrc?: string }>) {
	const { primaryHue, isDark } = useThemeInfo();
	const gradient = useMemo(
		() => hashToGradient(id, primaryHue, isDark),
		[id, primaryHue, isDark],
	);
	const icon = useAssetImage(iconUrl);
	const hasArtwork = icon.canRender || !!iconSrc;

	return (
		<div
			className="relative size-14 shrink-0 overflow-hidden rounded-xl border border-border/60 bg-muted sm:size-16"
			style={
				hasArtwork
					? undefined
					: {
							background: `linear-gradient(${gradient.angle}deg, ${gradient.from}, ${gradient.to})`,
						}
			}
		>
			{icon.canRender ? (
				<img
					ref={icon.imgRef}
					src={icon.src}
					onLoad={icon.onLoad}
					onError={icon.onError}
					alt=""
					className="size-full object-cover"
				/>
			) : iconSrc ? (
				<img src={iconSrc} alt="" className="size-full object-cover" />
			) : (
				<span className="flex size-full items-center justify-center font-mono text-lg font-bold text-white/70">
					{getPackageInitials(name)}
				</span>
			)}
		</div>
	);
}

function VersionLine({
	live,
	disk,
}: Readonly<{ live?: string; disk?: string }>) {
	const { t } = useTranslation("common");
	if (!live && !disk) return null;
	const value = (version: string) => (
		<span className="text-foreground">{version}</span>
	);
	return (
		<span className="inline-flex shrink-0 items-center gap-1.5 font-mono text-xs text-muted-foreground">
			{disk && live && disk !== live ? (
				<>
					{value(disk)}
					{t("workspaceOnDisk", "on disk")}
					<ArrowRight className="size-3" />
					{value(live)}
					{t("workspaceLive", "live")}
				</>
			) : live ? (
				<>
					{value(live)}
					{t("workspaceLive", "live")}
					{disk && ` · ${t("workspaceInSyncWithDisk", "in sync with disk")}`}
				</>
			) : (
				<>
					{value(disk ?? "")}
					{t("workspaceOnDisk", "on disk")}
				</>
			)}
		</span>
	);
}

const MUTED_TONE = {
	pill: "bg-muted text-muted-foreground",
	dot: "bg-muted-foreground",
};
const WARNING_TONE = {
	pill: "bg-tertiary/10 text-tertiary",
	dot: "bg-tertiary",
};

function DefaultStatePill({
	entry,
	access,
}: Readonly<{ entry?: RegistryEntry; access: ShellAccess }>) {
	const { t } = useTranslation("common");
	if (access.mode === "full" && entry)
		return <RegistryStatePill state={registryState(entry)} />;
	if (access.mode === "local_only")
		return (
			<StatePill tone={MUTED_TONE}>{t("localOnly", "Local only")}</StatePill>
		);
	if (access.mode === "local" && access.banner === "id_taken")
		return (
			<StatePill tone={WARNING_TONE}>
				{t("workspaceIdTaken", "ID taken")}
			</StatePill>
		);
	if (access.mode === "local" && access.banner === "signed_out")
		return (
			<StatePill tone={MUTED_TONE}>
				{t("workspaceSignedOut", "Signed out")}
			</StatePill>
		);
	if (access.mode === "local" && access.banner === "session_expired")
		return (
			<StatePill tone={MUTED_TONE}>
				{t("workspaceSessionExpired", "Session expired")}
			</StatePill>
		);
	return (
		<StatePill tone={MUTED_TONE}>
			{t("workspaceRegistryOffline", "Registry offline")}
		</StatePill>
	);
}

function WorkspaceHeader({
	access,
	entry,
	id,
	name,
	iconUrl,
	local,
	storeHref,
}: Readonly<{
	access: ShellAccess;
	entry?: RegistryEntry;
	id: string;
	name: string;
	iconUrl?: string;
	local?: PackageWorkspaceLocal;
	storeHref?: string;
}>) {
	const { t } = useTranslation("common");
	const managed = access.mode === "full" ? entry : undefined;
	const live = managed
		? liveVersion(asArray(managed.versions))?.version
		: undefined;

	return (
		<header className="mt-4 flex flex-col gap-4 md:flex-row md:items-center">
			<div className="flex min-w-0 flex-1 items-center gap-4">
				<WorkspaceIcon
					id={id || name}
					name={name}
					iconUrl={iconUrl}
					iconSrc={local?.header?.iconSrc}
				/>
				<div className="min-w-0 flex-1">
					<h1 className="truncate text-2xl font-semibold tracking-tight">
						{name}
					</h1>
					<div className="mt-2 flex min-w-0 flex-wrap items-center gap-2">
						{id && (
							<span className="truncate font-mono text-xs text-muted-foreground">
								{id}
							</span>
						)}
						{local?.header?.badges}
						{managed && (
							<span className="inline-flex h-5.5 shrink-0 items-center rounded-full border border-border/70 px-2 text-xs">
								<VisibilityLabel visibility={managed.visibility} />
							</span>
						)}
						{local?.header?.state ?? (
							<DefaultStatePill entry={entry} access={access} />
						)}
						<VersionLine live={live} disk={local?.header?.version} />
					</div>
				</div>
			</div>
			<div className="flex shrink-0 flex-wrap items-center gap-2">
				{local?.header?.actions}
				{storeHref && (
					<Button variant="outline" asChild>
						<Link href={storeHref}>
							<ExternalLink className="size-4" />
							{t("viewStorePage", "View store page")}
						</Link>
					</Button>
				)}
				{local?.primaryAction}
			</div>
		</header>
	);
}

function WorkspaceBannerNotice({
	banner,
	id,
	managed,
	onRetry,
	onOpenManifest,
	onSignIn,
}: Readonly<{
	banner: WorkspaceBanner;
	id: string;
	/** The owner shell is open, so the copy must not claim only the checkout shows. */
	managed: boolean;
	onRetry: () => void;
	onOpenManifest?: () => void;
	onSignIn?: () => void;
}>) {
	const { t } = useTranslation("common");
	const idTaken = banner === "id_taken";
	const needsSignIn = banner === "signed_out" || banner === "session_expired";
	const Icon = idTaken ? AlertTriangle : needsSignIn ? LogIn : CloudOff;
	const copy: Record<WorkspaceBanner, { title: string; description: string }> =
		{
			id_taken: {
				title: t("workspaceIdTakenTitle", "This id is taken on the registry"),
				description: t(
					"workspaceIdTakenDescription",
					"{{id}} belongs to another publisher, so the registry tabs stay hidden. Choose your own id in the manifest before you publish.",
					{ id },
				),
			},
			registry_unavailable: {
				title: t("workspaceRegistryUnavailableTitle", "Registry unavailable"),
				description: t(
					"workspaceRegistryUnavailableDescription",
					"Showing this checkout only. Registry details come back once the registry can be reached.",
				),
			},
			signed_out: {
				title: t("workspaceSignedOutTitle", "Sign in to see registry details"),
				description: t(
					"workspaceSignedOutDescription",
					"Showing this checkout only. Sign in to check whether you manage {{id}} on the registry.",
					{ id },
				),
			},
			session_expired: {
				title: t("workspaceSessionExpiredTitle", "Your session expired"),
				description: managed
					? t(
							"workspaceSessionExpiredDescription",
							"Registry details may be out of date, and changes won't save until you sign in again.",
						)
					: t(
							"workspaceSessionExpiredLocalDescription",
							"Showing this checkout only. Sign in again to load registry details.",
						),
			},
		};
	const { title, description } = copy[banner];

	return (
		<section
			aria-label={title}
			className={cn(
				"mt-5 flex flex-col gap-3 rounded-xl border px-4 py-3 sm:flex-row sm:items-center",
				idTaken
					? "border-tertiary/40 bg-tertiary/5"
					: "border-border/60 bg-muted/40",
			)}
		>
			<Icon
				className={cn(
					"size-5 shrink-0",
					idTaken ? "text-tertiary" : "text-muted-foreground",
				)}
			/>
			<div className="min-w-0 flex-1 text-sm">
				<p className="font-medium">{title}</p>
				<p className="text-muted-foreground">{description}</p>
			</div>
			<div className="flex shrink-0 flex-wrap items-center gap-2">
				{idTaken && onOpenManifest && (
					<Button size="sm" onClick={onOpenManifest}>
						{t("workspaceOpenManifest", "Open Manifest")}
					</Button>
				)}
				{banner === "registry_unavailable" && (
					<Button size="sm" variant="outline" onClick={onRetry}>
						<RotateCw className="size-3.5" />
						{t("retry", "Retry")}
					</Button>
				)}
				{needsSignIn && onSignIn && (
					<Button size="sm" onClick={onSignIn}>
						<LogIn className="size-3.5" />
						{t("signIn", "Sign in")}
					</Button>
				)}
			</div>
		</section>
	);
}

const TAB_TRIGGER_CLASS =
	"-mb-px h-10 flex-none rounded-none border-0 border-b-2 border-transparent bg-transparent px-0.5 text-[13px] text-muted-foreground shadow-none hover:text-foreground data-[state=active]:border-primary data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none dark:text-muted-foreground dark:data-[state=active]:border-primary dark:data-[state=active]:bg-transparent dark:data-[state=active]:text-foreground";

function WorkspaceShell({
	access,
	sessionExpired,
	entry,
	packageId,
	fetcher,
	auth,
	local,
	desktop,
	onRetry,
	onSignIn,
}: Readonly<{
	access: ShellAccess;
	sessionExpired: boolean;
	entry?: RegistryEntry;
	packageId?: string;
	fetcher: GenericFetcher;
	auth?: RegistryPackageAuth;
	local?: PackageWorkspaceLocal;
	desktop?: PackageWorkspaceDesktop;
	onRetry: () => void;
	onSignIn?: () => void;
}>) {
	const { t } = useTranslation("common");
	const router = useRouter();
	const pathname = usePathname();
	const searchParams = useSearchParams();
	const queryClient = useQueryClient();

	const full = access.mode === "full" ? access : undefined;
	const managed = full ? entry : undefined;
	const localBanner = access.mode === "local" ? access.banner : undefined;
	const banner =
		localBanner ?? (full && sessionExpired ? "session_expired" : undefined);
	const meta = usePackageMeta(managed?.id, fetcher, auth);
	const health = useMemo(
		() =>
			managed && meta.data !== undefined
				? listingHealth(managed, meta.data)
				: undefined,
		[managed, meta.data],
	);

	const tabs = useMemo(
		() =>
			workspaceTabs({
				hasLocal: !!local,
				hasRemote: !!managed,
				isOwner: full?.isOwner ?? false,
				isMaintainer: !!full,
				canLinkLocal: !!desktop?.linkFolder,
			}),
		[local, managed, full, desktop?.linkFolder],
	);
	const requested = workspaceTabFromParam(searchParams.get("tab"));
	const active: WorkspaceTab = tabs.includes(requested)
		? requested
		: "overview";

	const replaceParams = useCallback(
		(edit: (params: URLSearchParams) => void) => {
			const params = new URLSearchParams(searchParams.toString());
			edit(params);
			const query = params.toString();
			router.replace(query ? `${pathname}?${query}` : pathname, {
				scroll: false,
			});
		},
		[pathname, router, searchParams],
	);

	const selectTab = useCallback(
		(tab: WorkspaceTab) =>
			replaceParams((params) => {
				if (tab === "overview") params.delete("tab");
				else params.set("tab", tab);
			}),
		[replaceParams],
	);

	const dismissPublished = useCallback(
		() => replaceParams((params) => params.delete("published")),
		[replaceParams],
	);

	const storeHref = useMemo(() => {
		if (!entry) return undefined;
		const params = new URLSearchParams(searchParams.toString());
		params.delete("published");
		const query = params.toString();
		return packageStoreHref({
			id: entry.id,
			from: query ? `${pathname}?${query}` : pathname,
		});
	}, [entry, pathname, searchParams]);

	const handleDeleted = useCallback(() => {
		if (managed) invalidatePackageLists(queryClient, managed.id);
		router.push(MINE_HREF);
	}, [managed, queryClient, router]);

	const id = packageId || local?.header?.id || "";
	const name =
		(managed && (meta.data?.name?.trim() || managed.manifest.name?.trim())) ||
		local?.header?.name ||
		id;
	const nodeCount =
		local?.nodeCount ?? (managed ? asArray(managed.nodes).length : undefined);
	const labels: Record<WorkspaceTab, string> = {
		overview: t("overview", "Overview"),
		nodes: t("nodes", "Nodes"),
		test: t("workspaceTabTest", "Test"),
		manifest: t("workspaceTabManifest", "Manifest"),
		listing: t("workspaceTabListing", "Listing"),
		access: t("workspaceTabAccess", "Access"),
		releases: t("workspaceTabReleases", "Releases"),
	};
	const counts: Partial<Record<WorkspaceTab, number>> = {
		nodes: nodeCount,
		releases: managed ? asArray(managed.versions).length : undefined,
	};

	return (
		<WorkspaceFrame>
			<WorkspaceBreadcrumb label={id || name} />
			<WorkspaceHeader
				access={access}
				entry={entry}
				id={id}
				name={name}
				iconUrl={managed ? (meta.data?.icon ?? undefined) : undefined}
				local={local}
				storeHref={storeHref}
			/>
			{banner && (
				<WorkspaceBannerNotice
					banner={banner}
					id={id}
					managed={!!managed}
					onRetry={onRetry}
					onOpenManifest={
						tabs.includes("manifest") ? () => selectTab("manifest") : undefined
					}
					onSignIn={onSignIn}
				/>
			)}
			<Tabs
				value={active}
				onValueChange={(value) => {
					if (isWorkspaceTab(value)) selectTab(value);
				}}
				className="mt-5 gap-5"
			>
				<TabsList
					aria-label={t("workspaceTabsLabel", "Package workspace")}
					className="h-auto w-full justify-start gap-6 overflow-x-auto rounded-none border-b border-border/60 bg-transparent p-0"
				>
					{tabs.map((tab) => (
						<TabsTrigger key={tab} value={tab} className={TAB_TRIGGER_CLASS}>
							{labels[tab]}
							{counts[tab] !== undefined && (
								<CountBadge value={counts[tab] ?? 0} />
							)}
							{tab === "listing" && health && !health.complete && (
								<>
									<span
										aria-hidden="true"
										title={t(
											"workspaceListingIncomplete",
											"Listing incomplete",
										)}
										className="size-1.5 rounded-full bg-tertiary"
									/>
									<span className="sr-only">
										{t("workspaceListingIncomplete", "Listing incomplete")}
									</span>
								</>
							)}
						</TabsTrigger>
					))}
				</TabsList>

				<TabsContent value="overview">
					<OverviewTab
						entry={managed}
						meta={meta.data}
						metaLoading={meta.isLoading}
						banner={localBanner}
						main={local?.overview?.main}
						aside={local?.overview?.aside}
						storeHref={managed ? storeHref : undefined}
						fetcher={fetcher}
						auth={auth}
						onSelectTab={selectTab}
					/>
				</TabsContent>

				<TabsContent value="nodes">
					{local?.nodes ??
						(managed ? (
							<NodesTab
								nodes={asArray(managed.nodes)}
								version={liveVersion(asArray(managed.versions))?.version}
							/>
						) : (
							<WorkspaceNotice
								icon={FileCode}
								title={t("workspaceNotPublishedYet", "Not published yet")}
								description={t(
									"workspaceNotPublishedNodes",
									"Registry nodes appear once a version of this package is published.",
								)}
							/>
						))}
				</TabsContent>

				{tabs.includes("test") && (
					<TabsContent value="test">
						{local?.test ?? desktop?.linkFolder}
					</TabsContent>
				)}
				{tabs.includes("manifest") && (
					<TabsContent value="manifest">
						{local?.manifest ?? desktop?.linkFolder}
					</TabsContent>
				)}

				{managed && full && (
					<>
						<TabsContent value="listing">
							<ListingTab
								entry={managed}
								meta={meta.data}
								metaLoading={meta.isLoading}
								isOwner={full.isOwner}
								published={searchParams.get("published") === "1"}
								onDismissPublished={dismissPublished}
								fetcher={fetcher}
								auth={auth}
							/>
						</TabsContent>
						<TabsContent value="access">
							<AccessTab
								packageId={managed.id}
								permission={managed.currentUserPermission ?? 0}
								fetcher={fetcher}
								auth={auth}
							/>
						</TabsContent>
						<TabsContent value="releases">
							<ReleasesTab
								entry={managed}
								isOwner={full.isOwner}
								canInstall={!!desktop?.canInstall}
								publishAction={local?.primaryAction}
								diskVersion={local?.header?.version}
								fetcher={fetcher}
								auth={auth}
								onDeleted={handleDeleted}
							/>
						</TabsContent>
					</>
				)}
			</Tabs>
		</WorkspaceFrame>
	);
}
