"use client";

import { useTranslation } from "@flow-like/locales";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	Ban,
	Package,
	Plus,
	RefreshCw,
	ShoppingBag,
	Trash2,
	TriangleAlert,
} from "lucide-react";
import Link from "next/link";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import { useInvoke } from "../../hooks/use-invoke";
import { isPurchaseRequiredError } from "../../lib/api-error";
import {
	type PackageAccess,
	groupPackageNodesByCategory,
	packageAccess,
} from "../../lib/app-package-overview";
import {
	type LicenseTimeLeft,
	type PackagePinState,
	licenseExpiresAt,
	licenseTimeLeft,
	packagePinState,
	storePackageHref,
} from "../../lib/package-license";
import type { AppPackageWidget } from "../../lib/package-widgets";
import { asArray } from "../../lib/response-shape";
import type { INode } from "../../lib/schema/flow/node";
import type {
	AddAppPackageRequest,
	AppPackage,
	InstalledPackage,
	PackageUpdate,
	UpdateAppPackageRequest,
} from "../../lib/schema/wasm";
import { useBackend } from "../../state/backend-state";
import { Alert, AlertDescription, AlertTitle } from "../ui/alert";
import { Badge } from "../ui/badge";
import { Button } from "../ui/button";
import { EmptyState } from "../ui/empty-state";
import { Skeleton } from "../ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../ui/tabs";
import {
	type PackageAccessRow,
	PackageAccessSection,
} from "./app-packages/package-access-section";
import {
	PackageNodeCategories,
	PackageNodeList,
} from "./app-packages/package-nodes-section";
import { PackageTile } from "./app-packages/package-tile";
import { PackageUpdatesBanner } from "./app-packages/package-updates-banner";
import { PackageWidgetsSection } from "./app-packages/package-widgets-section";
import { LICENSE_WARNING_BADGE_CLASS } from "./app-packages/parts";
import {
	APP_PACKAGE_MANIFEST_KEY,
	type PackageManifestView,
	usePackageManifests,
} from "./app-packages/use-package-manifests";
import { PackageSearchDialog } from "./package-search-dialog";
import {
	WidgetPermissionsButton,
	WidgetPermissionsSheet,
	useMicroWidgetConsentEntries,
} from "./widget-permissions";

export interface AppPackagesPageProps {
	appId: string;
}

function installedToAppPackage(
	pkg: InstalledPackage,
	pinnedVersion: string,
): AppPackage {
	return {
		id: pkg.id,
		appId: "",
		packageId: pkg.id,
		packageName: pkg.manifest.name,
		version: pinnedVersion,
		autoUpdate: false,
		addedAt: pkg.installedAt,
		stale: false,
		metadata: pkg.metadata,
	};
}

interface PinLicenseView {
	state: PackagePinState;
	expiresAt?: number;
	timeLeft?: LicenseTimeLeft;
}

type PackagesTab = "overview" | "nodes" | "widgets" | "access";

interface PackageView {
	pkg: AppPackage;
	name: string;
	description?: string;
	nodes: INode[];
	manifest?: PackageManifestView;
	license: PinLicenseView;
	timeLeftLabel?: string;
	access: PackageAccess;
}

const LICENSE_TICK_MS = 60_000;
const OVERVIEW_WIDGET_LIMIT = 3;
const SKELETON_KEYS = ["a", "b", "c", "d"] as const;
const WARNING_ALERT_CLASS =
	"border-amber-500/40 bg-amber-500/5 [&>svg]:text-amber-600 dark:[&>svg]:text-amber-400";

function useNow(intervalMs: number, enabled: boolean): number {
	const [now, setNow] = useState(() => Date.now());
	useEffect(() => {
		if (!enabled) return;
		setNow(Date.now());
		const id = setInterval(() => setNow(Date.now()), intervalMs);
		return () => clearInterval(id);
	}, [intervalMs, enabled]);
	return now;
}

/** Servers without licensing leave the decision to the reactivate endpoint. */
function canReactivate(pkg: AppPackage): boolean {
	return pkg.license ? pkg.viewerHasPackage === true : true;
}

function pinLicenseView(pkg: AppPackage, now: number): PinLicenseView {
	const state = packagePinState(pkg, now);
	const expiresAt = licenseExpiresAt(pkg);
	return {
		state,
		expiresAt,
		timeLeft: state === "lapsed" ? licenseTimeLeft(expiresAt, now) : undefined,
	};
}

function useTimeLeftLabel() {
	const { t } = useTranslation("store");
	return useCallback(
		(left: LicenseTimeLeft) =>
			left.days >= 1
				? t("licenseDaysLeft", {
						defaultValue_one: "{{count}} day left",
						defaultValue_other: "{{count}} days left",
						count: left.days,
					})
				: t("licenseHoursLeft", {
						defaultValue_one: "{{count}} hour left",
						defaultValue_other: "{{count}} hours left",
						count: left.hours,
					}),
		[t],
	);
}

export function AppPackagesPage({ appId }: AppPackagesPageProps) {
	const { t } = useTranslation("store");
	const backend = useBackend();
	const router = useRouter();
	const queryClient = useQueryClient();
	const [searchOpen, setSearchOpen] = useState(false);
	const [permissionsOpen, setPermissionsOpen] = useState(false);
	const [tab, setTab] = useState<PackagesTab>("overview");
	const widgetConsents = useMicroWidgetConsentEntries({ appId });
	const formatTimeLeft = useTimeLeftLabel();

	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const isOffline = useQuery<boolean>({
		queryKey: ["app-offline", appId],
		queryFn: () => backend.isOffline(appId),
		enabled: !!appId,
	});

	const packages = useQuery<AppPackage[]>({
		queryKey: ["app", appId, "packages"],
		queryFn: async () => {
			if (isOffline.data && backend.appState.listPackages) {
				const pkgMap = await backend.appState.listPackages(appId);
				const installed = await backend.registryState.getInstalledPackages();
				const installedMap = new Map(installed.map((p) => [p.id, p]));
				return Object.entries(pkgMap).map(([pkgId, version]) => {
					const local = installedMap.get(pkgId);
					if (local) return installedToAppPackage(local, version);
					return {
						id: pkgId,
						appId,
						packageId: pkgId,
						packageName: pkgId,
						version,
						autoUpdate: false,
						addedAt: new Date().toISOString(),
						stale: false,
					} satisfies AppPackage;
				});
			}
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<AppPackage[]>(
				profile.data,
				`apps/${appId}/packages`,
			);
		},
		enabled:
			!!appId &&
			isOffline.data !== undefined &&
			(isOffline.data || !!profile.data),
	});
	const packageRows = useMemo(() => asArray(packages.data), [packages.data]);

	const catalog = useQuery<INode[]>({
		queryKey: ["app-catalog-nodes", appId],
		queryFn: () => backend.boardState.getCatalog(appId),
		enabled: !!appId && packageRows.length > 0,
	});

	const updates = useQuery<PackageUpdate[]>({
		queryKey: ["app", appId, "package-updates"],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<PackageUpdate[]>(
				profile.data,
				`apps/${appId}/packages/updates`,
			);
		},
		enabled:
			!!appId &&
			isOffline.data === false &&
			!!profile.data &&
			packageRows.length > 0,
	});

	const nodesByPackage = useMemo(() => {
		if (!catalog.data) return new Map<string, INode[]>();
		const map = new Map<string, INode[]>();
		for (const node of catalog.data) {
			if (!node.wasm?.package_id) continue;
			const existing = map.get(node.wasm.package_id);
			if (existing) {
				existing.push(node);
			} else {
				map.set(node.wasm.package_id, [node]);
			}
		}
		return map;
	}, [catalog.data]);

	const updatesByPackage = useMemo(() => {
		const map = new Map<string, PackageUpdate>();
		for (const update of asArray(updates.data)) {
			map.set(update.packageId, update);
		}
		return map;
	}, [updates.data]);

	const invalidatePackageQueries = useCallback(() => {
		queryClient.invalidateQueries({ queryKey: ["app", appId, "packages"] });
		queryClient.invalidateQueries({
			queryKey: ["app", appId, "package-updates"],
		});
		queryClient.invalidateQueries({ queryKey: ["app-catalog-nodes", appId] });
		queryClient.invalidateQueries({ queryKey: ["getCatalog", appId] });
		queryClient.invalidateQueries({ queryKey: ["app-package-widgets", appId] });
		queryClient.invalidateQueries({ queryKey: [APP_PACKAGE_MANIFEST_KEY] });
	}, [queryClient, appId]);

	const addPackage = useMutation({
		mutationFn: async (req: AddAppPackageRequest) => {
			if (isOffline.data && backend.appState.addPackage) {
				await backend.appState.addPackage(appId, req.packageId, req.version);
				return;
			}
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.post(profile.data, `apps/${appId}/packages`, req);
		},
		onSuccess: () => {
			toast.success(t("packageAdded", "Package added"));
			invalidatePackageQueries();
		},
		onError: (err: Error, req) => {
			if (isPurchaseRequiredError(err)) {
				toast.error(
					t(
						"packageLicenseRequiredToAdd",
						"Get this package before adding it. An admin or the owner who holds a paid package licenses it for the project.",
					),
					{
						action: {
							label: t("getPackage", "Get package"),
							onClick: () => router.push(storePackageHref(req.packageId)),
						},
					},
				);
				return;
			}
			toast.error(
				t("failedToAddPackageMessage", "Failed to add package: {{message}}", {
					message: err.message,
				}),
			);
		},
	});

	const removePackage = useMutation({
		mutationFn: async (pkgId: string) => {
			if (isOffline.data && backend.appState.removePackage) {
				await backend.appState.removePackage(appId, pkgId);
				return;
			}
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.del(
				profile.data,
				`apps/${appId}/packages/${pkgId}`,
			);
		},
		onSuccess: () => {
			toast.success(t("packageRemoved", "Package removed"));
			invalidatePackageQueries();
		},
		onError: (err: Error) =>
			toast.error(
				t(
					"failedToRemovePackageMessage",
					"Failed to remove package: {{message}}",
					{ message: err.message },
				),
			),
	});

	const toggleAutoUpdate = useMutation({
		mutationFn: async ({
			pkgId,
			autoUpdate,
		}: { pkgId: string; autoUpdate: boolean }) => {
			if (isOffline.data) return;
			if (!profile.data) throw new Error("Profile not loaded");
			const body: UpdateAppPackageRequest = { autoUpdate };
			return backend.apiState.patch(
				profile.data,
				`apps/${appId}/packages/${pkgId}`,
				body,
			);
		},
		onSuccess: () => {
			if (!isOffline.data) {
				toast.success(t("autoUpdateToggled", "Auto-update toggled"));
				queryClient.invalidateQueries({ queryKey: ["app", appId, "packages"] });
			}
		},
		onError: (err: Error) =>
			toast.error(
				t(
					"failedToUpdatePackageMessage",
					"Failed to update package: {{message}}",
					{ message: err.message },
				),
			),
	});

	const reactivatePackage = useMutation({
		mutationFn: async (pkgId: string) => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.post(
				profile.data,
				`apps/${appId}/packages/${pkgId}/reactivate`,
				{},
			);
		},
		onSuccess: () => {
			toast.success(t("packageReactivated", "Package reactivated"));
			invalidatePackageQueries();
		},
		onError: (err: Error) =>
			toast.error(
				t("failedToReactivateMessage", "Failed to reactivate: {{message}}", {
					message: err.message,
				}),
			),
	});

	const patchPackageVersion = useCallback(
		async (pkgId: string, version: string) => {
			if (!profile.data) throw new Error("Profile not loaded");
			const body: UpdateAppPackageRequest = { version };
			return backend.apiState.patch(
				profile.data,
				`apps/${appId}/packages/${pkgId}`,
				body,
			);
		},
		[backend.apiState, profile.data, appId],
	);

	const applyUpdate = useMutation({
		mutationFn: ({ pkgId, version }: { pkgId: string; version: string }) =>
			patchPackageVersion(pkgId, version),
		onSuccess: () => {
			toast.success(t("packageUpdated", "Package updated"));
			invalidatePackageQueries();
		},
		onError: (err: Error) =>
			toast.error(
				t(
					"failedToUpdatePackageMessage",
					"Failed to update package: {{message}}",
					{ message: err.message },
				),
			),
	});

	const applyAllUpdates = useMutation({
		mutationFn: async (updatesToApply: PackageUpdate[]) => {
			const results = await Promise.allSettled(
				updatesToApply.map((update) =>
					patchPackageVersion(update.packageId, update.latestVersion),
				),
			);
			const failed = results.filter((r) => r.status === "rejected").length;
			return { total: results.length, failed };
		},
		onSuccess: ({ total, failed }) => {
			if (failed === 0) {
				toast.success(
					t("countPackagesUpdated", {
						defaultValue_one: "Package updated",
						defaultValue_other: "{{count}} packages updated",
						count: total,
					}),
				);
			} else {
				toast.error(`Failed to update ${failed} of ${total} packages`);
			}
			invalidatePackageQueries();
		},
	});

	const handleSelect = useCallback(
		(packageId: string, version: string) =>
			addPackage.mutateAsync({
				packageId,
				version,
				autoUpdate: !isOffline.data,
			}),
		[addPackage, isOffline.data],
	);

	// Updates that can actually be applied: package is not stale and has a
	// newer version available. Stale packages must be reactivated first.
	const applicableUpdates = useMemo(
		() =>
			packageRows
				.filter((p) => !p.stale)
				.flatMap((p) => updatesByPackage.get(p.packageId) ?? []),
		[packageRows, updatesByPackage],
	);

	const packageNames = useMemo(
		() =>
			new Map(
				packageRows.map((p) => [p.packageId, p.packageName ?? p.packageId]),
			),
		[packageRows],
	);

	const hasLapsedPins = useMemo(
		() => packageRows.some((p) => p.license?.status === "lapsed"),
		[packageRows],
	);
	const now = useNow(LICENSE_TICK_MS, hasLapsedPins);
	const licenseViews = useMemo(
		() =>
			new Map(packageRows.map((p) => [p.packageId, pinLicenseView(p, now)])),
		[packageRows, now],
	);
	const licenseAlerts = useMemo(
		() =>
			packageRows.flatMap((pkg) => {
				const view = licenseViews.get(pkg.packageId);
				return view && (view.state === "lapsed" || view.state === "expired")
					? [{ pkg, view }]
					: [];
			}),
		[packageRows, licenseViews],
	);

	const excludeIds = packageRows.map((p) => p.packageId);

	const packageIds = useMemo(
		() => packageRows.map((p) => p.packageId),
		[packageRows],
	);
	const manifests = usePackageManifests(packageIds);
	const offline = !!isOffline.data;
	const countsLoading = catalog.isLoading;

	const views = useMemo(
		(): PackageView[] =>
			packageRows.map((pkg) => {
				const manifest = manifests.byId.get(pkg.packageId);
				const nodes = nodesByPackage.get(pkg.packageId) ?? [];
				const license =
					licenseViews.get(pkg.packageId) ?? pinLicenseView(pkg, now);
				return {
					pkg,
					name: pkg.metadata?.name || pkg.packageName || pkg.packageId,
					description:
						pkg.metadata?.description || manifest?.access?.description,
					nodes,
					manifest,
					license,
					timeLeftLabel: license.timeLeft
						? formatTimeLeft(license.timeLeft)
						: undefined,
					access: packageAccess(nodes, manifest?.access),
				};
			}),
		[
			packageRows,
			manifests.byId,
			nodesByPackage,
			licenseViews,
			now,
			formatTimeLeft,
		],
	);

	const displayNames = useMemo(
		() => new Map(views.map((view) => [view.pkg.packageId, view.name])),
		[views],
	);

	const widgets = useMemo(
		(): AppPackageWidget[] =>
			views.flatMap((view) =>
				(view.manifest?.widgets ?? []).map((widget) => ({
					packageId: view.pkg.packageId,
					packageName: view.name,
					packageVersion: view.pkg.version,
					bundleHash: view.manifest?.bundleHash,
					widget,
				})),
			),
		[views],
	);

	const nodeGroups = useMemo(
		() =>
			groupPackageNodesByCategory(
				new Map(views.map((view) => [view.pkg.packageId, view.nodes])),
				t("appPackagesUncategorized", "Uncategorized"),
			),
		[views, t],
	);
	const nodeTotal = useMemo(
		() => views.reduce((sum, view) => sum + view.nodes.length, 0),
		[views],
	);

	const mutedPackageIds = useMemo(
		() =>
			new Set(
				views
					.filter(
						(view) =>
							view.license.state === "stale" ||
							view.license.state === "expired",
					)
					.map((view) => view.pkg.packageId),
			),
		[views],
	);

	const accessRows = useMemo(
		(): PackageAccessRow[] =>
			views.map((view) => ({
				packageId: view.pkg.packageId,
				name: view.name,
				access: view.access,
				widgets: view.manifest?.widgets ?? [],
				pinState: view.license.state,
				timeLeftLabel: view.timeLeftLabel,
			})),
		[views],
	);

	const openGrants = useCallback(() => setPermissionsOpen(true), []);
	const showTab = useCallback((next: PackagesTab) => () => setTab(next), []);

	if (packages.isLoading || isOffline.isLoading)
		return <PackagesPageSkeleton />;

	const summary =
		packageRows.length > 0 && !countsLoading
			? t("appPackagesSummary", {
					defaultValue_one:
						"{{count}} package gives this app {{nodes}} and {{widgets}}.",
					defaultValue_other:
						"{{count}} packages give this app {{nodes}} and {{widgets}}.",
					count: packageRows.length,
					nodes: t("nodeCount", {
						defaultValue_one: "{{count}} node",
						defaultValue_other: "{{count}} nodes",
						count: nodeTotal,
					}),
					widgets: t("appPackagesWidgetCount", {
						defaultValue_one: "{{count}} widget",
						defaultValue_other: "{{count}} widgets",
						count: widgets.length,
					}),
				})
			: t("wasmPackagesLinkedToThisApp", "WASM packages linked to this app");

	const accessSection = (
		<PackageAccessSection
			rows={accessRows}
			grantCount={widgetConsents.length}
			onManageGrants={openGrants}
		/>
	);

	return (
		<div className="flex flex-col gap-6">
			<header className="flex flex-wrap items-start justify-between gap-4">
				<div className="min-w-0">
					<h1 className="text-xl font-semibold tracking-tight">
						{t("packages", "Packages")}
					</h1>
					<p className="mt-1 text-sm text-muted-foreground">{summary}</p>
				</div>
				<div className="flex flex-wrap items-center gap-2">
					<WidgetPermissionsButton
						count={widgetConsents.length}
						onClick={openGrants}
					/>
					<Button size="sm" onClick={() => setSearchOpen(true)}>
						<Plus className="size-4" />
						{t("addPackage", "Add Package")}
					</Button>
				</div>
			</header>

			{!offline &&
				licenseAlerts.map(({ pkg, view }) => (
					<PackageLicenseAlert
						key={pkg.id}
						pkg={pkg}
						view={view}
						onReactivate={() => reactivatePackage.mutate(pkg.packageId)}
						onRemove={() => removePackage.mutate(pkg.packageId)}
						isReactivating={reactivatePackage.isPending}
						isRemoving={removePackage.isPending}
					/>
				))}

			{packageRows.length === 0 ? (
				<EmptyState
					className="w-full max-w-none grow"
					icons={[Package]}
					title={t("noPackages", "No packages")}
					description={t(
						"addAWasmPackageToGetStarted",
						"Add a WASM package to get started.",
					)}
				/>
			) : (
				<Tabs
					value={tab}
					onValueChange={(value) => setTab(value as PackagesTab)}
					className="gap-6"
				>
					<TabsList>
						<TabsTrigger value="overview">
							{t("overview", "Overview")}
						</TabsTrigger>
						<TabsTrigger value="nodes">
							{t("appPackagesNodes", "Nodes")}
							{!countsLoading && <TabCount count={nodeTotal} />}
						</TabsTrigger>
						<TabsTrigger value="widgets">
							{t("appPackagesWidgets", "Widgets")}
							{!manifests.loading && <TabCount count={widgets.length} />}
						</TabsTrigger>
						<TabsTrigger value="access">
							{t("appPackagesAccess", "Access")}
						</TabsTrigger>
					</TabsList>

					<TabsContent value="overview" className="flex flex-col gap-8">
						{!offline && (
							<PackageUpdatesBanner
								updates={applicableUpdates}
								packageNames={displayNames}
								pending={applyUpdate.isPending || applyAllUpdates.isPending}
								onApply={(update) =>
									applyUpdate.mutate({
										pkgId: update.packageId,
										version: update.latestVersion,
									})
								}
								onApplyAll={() => applyAllUpdates.mutate(applicableUpdates)}
							/>
						)}
						<section
							aria-label={t("appPackagesLinked", "Linked packages")}
							className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4"
						>
							{views.map((view) => (
								<PackageTile
									key={view.pkg.id}
									pkg={view.pkg}
									name={view.name}
									description={view.description}
									pinState={view.license.state}
									timeLeftLabel={view.timeLeftLabel}
									nodeCount={view.nodes.length}
									widgetCount={view.manifest?.widgets.length ?? 0}
									countsLoading={countsLoading}
									access={view.access}
									update={updatesByPackage.get(view.pkg.packageId)}
									offline={offline}
									canReactivate={canReactivate(view.pkg)}
									pending={{
										autoUpdate: toggleAutoUpdate.isPending,
										update: applyUpdate.isPending || applyAllUpdates.isPending,
										reactivate: reactivatePackage.isPending,
										remove: removePackage.isPending,
									}}
									actions={{
										onToggleAutoUpdate: (autoUpdate) =>
											toggleAutoUpdate.mutate({
												pkgId: view.pkg.packageId,
												autoUpdate,
											}),
										onApplyUpdate: () => {
											const update = updatesByPackage.get(view.pkg.packageId);
											if (update) {
												applyUpdate.mutate({
													pkgId: view.pkg.packageId,
													version: update.latestVersion,
												});
											}
										},
										onReactivate: () =>
											reactivatePackage.mutate(view.pkg.packageId),
										onRemove: () => removePackage.mutate(view.pkg.packageId),
									}}
								/>
							))}
						</section>
						<PackageWidgetsSection
							widgets={widgets}
							loading={manifests.loading}
							limit={OVERVIEW_WIDGET_LIMIT}
							onShowAll={showTab("widgets")}
						/>
						<PackageNodeCategories
							groups={nodeGroups}
							packageNames={displayNames}
							mutedPackageIds={mutedPackageIds}
							loading={countsLoading}
							onShowAll={showTab("nodes")}
						/>
						{accessSection}
					</TabsContent>

					<TabsContent value="nodes">
						<PackageNodeList
							groups={nodeGroups}
							packageNames={displayNames}
							mutedPackageIds={mutedPackageIds}
							loading={countsLoading}
						/>
					</TabsContent>

					<TabsContent value="widgets">
						<PackageWidgetsSection
							widgets={widgets}
							loading={manifests.loading}
						/>
					</TabsContent>

					<TabsContent value="access">{accessSection}</TabsContent>
				</Tabs>
			)}

			<PackageSearchDialog
				open={searchOpen}
				onOpenChange={setSearchOpen}
				onSelect={handleSelect}
				onRemove={(id) => removePackage.mutateAsync(id)}
				excludePackageIds={excludeIds}
				appId={appId}
			/>
			<WidgetPermissionsSheet
				appId={appId}
				entries={widgetConsents}
				packageNames={packageNames}
				open={permissionsOpen}
				onOpenChange={setPermissionsOpen}
			/>
		</div>
	);
}

function TabCount({ count }: Readonly<{ count: number }>) {
	return (
		<span className="rounded-full bg-muted px-1.5 text-[11px] tabular-nums text-muted-foreground">
			{count}
		</span>
	);
}

function PackageLicenseAlert({
	pkg,
	view,
	onReactivate,
	onRemove,
	isReactivating,
	isRemoving,
}: Readonly<{
	pkg: AppPackage;
	view: PinLicenseView;
	onReactivate: () => void;
	onRemove: () => void;
	isReactivating: boolean;
	isRemoving: boolean;
}>) {
	const { t, i18n } = useTranslation("store");
	const formatTimeLeft = useTimeLeftLabel();
	const language = i18n.resolvedLanguage ?? i18n.language;
	const disabledOn = useMemo(
		() =>
			view.expiresAt === undefined
				? undefined
				: new Intl.DateTimeFormat(language, {
						dateStyle: "medium",
						timeStyle: "short",
					}).format(view.expiresAt),
		[view.expiresAt, language],
	);
	const name = pkg.packageName ?? pkg.packageId;
	const expired = view.state === "expired";

	return (
		<Alert
			variant={expired ? "destructive" : "default"}
			className={expired ? "" : WARNING_ALERT_CLASS}
		>
			{expired ? (
				<Ban className="h-4 w-4" />
			) : (
				<TriangleAlert className="h-4 w-4" />
			)}
			<AlertTitle className="line-clamp-none flex flex-wrap items-center gap-2">
				{expired
					? t(
							"packageDisabledInProject",
							"{{name}} is disabled in this project",
							{
								name,
							},
						)
					: t("packageLicenseLapsedTitle", "{{name}} is no longer licensed", {
							name,
						})}
				{view.timeLeft && (
					<Badge
						variant="outline"
						className={`text-xs ${LICENSE_WARNING_BADGE_CLASS}`}
					>
						{formatTimeLeft(view.timeLeft)}
					</Badge>
				)}
			</AlertTitle>
			<AlertDescription className="gap-3">
				<p>
					{expired
						? t(
								"packageLicenseExpiredDescription",
								"No admin or owner of this project has this package, so its licence expired. Cloud runs and downloads of it have stopped. Get the package and reactivate it to restore it, or remove it from the project.",
							)
						: disabledOn
							? t(
									"packageLicenseLapsedDescription",
									"No admin or owner of this project has this package. Updates are paused and it will be disabled on {{date}}. One of them needs to get the package to keep it working.",
									{ date: disabledOn },
								)
							: t(
									"packageLicenseLapsedDescriptionUndated",
									"No admin or owner of this project has this package. Updates are paused and it will be disabled soon. One of them needs to get the package to keep it working.",
								)}
				</p>
				<div className="flex flex-wrap items-center gap-2">
					<Button asChild size="sm" variant={expired ? "default" : "outline"}>
						<Link href={storePackageHref(pkg.packageId)}>
							<ShoppingBag className="mr-1 h-3.5 w-3.5" />
							{t("getPackage", "Get package")}
						</Link>
					</Button>
					<Button
						size="sm"
						variant="outline"
						onClick={onReactivate}
						disabled={isReactivating || !canReactivate(pkg)}
					>
						<RefreshCw className="mr-1 h-3.5 w-3.5" />
						{t("reactivate", "Reactivate")}
					</Button>
					{expired && (
						<Button
							size="sm"
							variant="ghost"
							onClick={onRemove}
							disabled={isRemoving}
						>
							<Trash2 className="mr-1 h-3.5 w-3.5 text-destructive" />
							{t("removePackage", "Remove package")}
						</Button>
					)}
					{!canReactivate(pkg) && (
						<span className="text-xs text-muted-foreground">
							{t(
								"getPackageToReactivate",
								"You don't have this package yet. Get it first, then reactivate it here.",
							)}
						</span>
					)}
				</div>
			</AlertDescription>
		</Alert>
	);
}

function PackagesPageSkeleton() {
	return (
		<div className="flex flex-col gap-6">
			<div className="flex items-start justify-between gap-4">
				<div className="space-y-2">
					<Skeleton className="h-6 w-28" />
					<Skeleton className="h-4 w-72" />
				</div>
				<Skeleton className="h-8 w-32" />
			</div>
			<Skeleton className="h-9 w-80" />
			<div className="grid gap-4 sm:grid-cols-2 xl:grid-cols-4">
				{SKELETON_KEYS.map((key) => (
					<Skeleton key={key} className="h-60 rounded-xl" />
				))}
			</div>
		</div>
	);
}
