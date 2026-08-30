"use client";

import {
	Breadcrumb,
	BreadcrumbItem,
	BreadcrumbLink,
	BreadcrumbList,
	BreadcrumbPage,
	BreadcrumbSeparator,
	Button,
	Card,
	CardContent,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	HoverCard,
	HoverCardContent,
	HoverCardTrigger,
	IAppVisibility,
	type IEvent,
	Input,
	Label,
	ScrollArea,
	Separator,
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
	Skeleton,
	Switch,
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
	VisibilityIcon,
	toastError,
	useBackend,
	useDeveloperMode,
	useExecutionServiceOptional,
	useInvoke,
	useMobileHeader,
} from "@flow-like/flow-like-ui";
import { AppPublicationBanner } from "@flow-like/flow-like-ui/components/settings/visibility-status/app-publication-banner";
import {
	type AppPublicationRequestItem,
	type RawAppPublicationRequestItem,
	normalizeAppPublicationRequests,
} from "@flow-like/flow-like-ui/components/settings/visibility-status/app-publication-review-card";
import { VisibilityUpgradeDialog } from "@flow-like/flow-like-ui/components/settings/visibility-status/visibility-upgrade-dialog";
import { configRouteFillsHeight } from "@flow-like/flow-like-ui/lib/config-route";
import { EVENT_CONFIG } from "@flow-like/flow-like-ui/lib/event-config";
import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { useLiveQuery } from "dexie-react-hooks";
import {
	ChartAreaIcon,
	ChevronLeftIcon,
	CogIcon,
	CopyIcon,
	CrownIcon,
	DatabaseIcon,
	DollarSignIcon,
	DownloadIcon,
	EyeIcon,
	EyeOffIcon,
	FolderClosedIcon,
	GlobeIcon,
	KeyIcon,
	LayersIcon,
	LayoutGridIcon,
	LockIcon,
	Maximize2Icon,
	MenuIcon,
	Minimize2Icon,
	PackageIcon,
	PlayCircleIcon,
	SearchIcon,
	SendIcon,
	SparklesIcon,
	SquarePenIcon,
	UnlockIcon,
	UserIcon,
	UsersRoundIcon,
	WorkflowIcon,
	ZapIcon,
} from "lucide-react";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import {
	Suspense,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import { toast } from "sonner";
import { appsDB } from "../../../lib/apps-db";
import { isIosTauriRuntime } from "../../../lib/platform";

interface INavigationItem {
	href: string;
	label: string;
	icon: React.ForwardRefExoticComponent<
		Omit<import("lucide-react").LucideProps, "ref"> &
			React.RefAttributes<SVGSVGElement>
	>;
	description: string;
	group: string;
	visibilities?: IAppVisibility[];
	requiresPaid?: boolean;
	disabled?: boolean;
	devOnly?: boolean;
	/**
	 * Visibilities where the section stays in the nav but is locked: hiding it
	 * outright reads as "this feature does not exist". Clicking a locked row
	 * offers the visibility change that unlocks it.
	 */
	lockedVisibilities?: IAppVisibility[];
	/** Copy for the unlock dialog. */
	lockedReason?: string;
	/** Visibility the unlock dialog switches to. */
	unlockVisibility?: IAppVisibility;
}

/** Labels are built per render so a language switch relabels the config nav. */
function buildNavigationItems(
	t: (key: string, defaultValue: string) => string,
): INavigationItem[] {
	const groups = {
		general: t("general", "General"),
		build: t("build", "Build"),
		data: t("data", "Data"),
		collaborate: t("collaborate", "Collaborate"),
		insights: t("insights", "Insights"),
	};

	return [
		{
			href: "/library/config",
			label: t("dashboard", "Dashboard"),
			icon: SquarePenIcon,
			description: t(
				"overviewStatsAndGettingStarted",
				"Overview, stats, and getting started",
			),
			group: groups.general,
		},
		{
			href: "/library/config/configuration",
			label: t("configuration", "Configuration"),
			icon: CogIcon,
			description: t(
				"appConfigurationAndEnvironmentVariables",
				"App configuration and environment variables",
			),
			group: groups.general,
		},
		{
			href: "/library/config/runtime-vars",
			label: t("runtimeVariables", "Runtime Variables"),
			icon: KeyIcon,
			description: t(
				"userspecificRuntimeSecretsAndConfigurations",
				"User-specific runtime secrets and configurations",
			),
			group: groups.general,
		},
		{
			href: "/library/config/flows",
			label: t("flows", "Flows"),
			icon: WorkflowIcon,
			description: t(
				"businessLogicAndWorkflowDefinitions",
				"Business logic and workflow definitions",
			),
			group: groups.build,
			devOnly: true,
		},
		{
			href: "/library/config/pages",
			label: t("events", "Events"),
			icon: SparklesIcon,
			description: t(
				"eventsPagesAndPathbasedNavigation",
				"Events, pages, and path-based navigation",
			),
			group: groups.build,
			devOnly: true,
		},
		{
			href: "/library/config/templates",
			label: t("templates", "Templates"),
			icon: CopyIcon,
			description: t("reusableFlowTemplates", "Reusable Flow templates"),
			group: groups.build,
			devOnly: true,
		},
		{
			href: "/library/config/widgets",
			label: t("widgets", "Widgets"),
			icon: LayoutGridIcon,
			description: t(
				"reusableUiComponentsAndWidgets",
				"Reusable UI components and widgets",
			),
			group: groups.build,
			devOnly: true,
		},
		{
			href: "/library/config/storage",
			label: t("storage", "Storage"),
			icon: FolderClosedIcon,
			description: t(
				"dataStorageAndFileManagement",
				"Data storage and file management",
			),
			group: groups.data,
		},
		{
			href: "/library/config/user-storage",
			label: t("userStorage", "User Storage"),
			icon: UserIcon,
			description: t(
				"browseAndSearchYourPrivateAppFiles",
				"Browse and search your private app files",
			),
			group: groups.data,
			devOnly: true,
		},
		{
			href: "/library/config/explore",
			label: t("dataStudio", "Data Studio"),
			icon: DatabaseIcon,
			description: t(
				"modelExploreOperateAndShareProjectData",
				"Model, explore, operate, and share project data",
			),
			group: groups.data,
			devOnly: true,
		},
		{
			href: "/library/config/packages",
			label: t("packages", "Packages"),
			icon: PackageIcon,
			description: t(
				"manageWasmPackagesForThisApp",
				"Manage WASM packages for this app",
			),
			group: groups.data,
			devOnly: true,
		},
		{
			href: "/library/config/team",
			label: t("team", "Team"),
			icon: UsersRoundIcon,
			description: t(
				"manageTeamMembersAndPermissions",
				"Manage team members and permissions",
			),
			visibilities: [
				IAppVisibility.Public,
				IAppVisibility.Prototype,
				IAppVisibility.PublicRequestAccess,
			],
			lockedVisibilities: [IAppVisibility.Private],
			lockedReason: t(
				"aPrivateProjectIsSyncedToYourAccountOnlySwitchToPrototypeToInviteCollaboratorsAssignRolesAndShareALink",
				"A private project is synced to your account only. Switch to Prototype to invite collaborators, assign roles and share a link.",
			),
			unlockVisibility: IAppVisibility.Prototype,
			group: groups.collaborate,
		},
		{
			href: "/library/config/suites",
			label: t("suites", "Suites"),
			icon: LayersIcon,
			description: t(
				"bundleThisAppWithRelatedAppsIntoOneStoreListing",
				"Bundle this app with related apps into one store listing",
			),
			// A suite is presentation, not membership — private apps curate them too.
			visibilities: [
				IAppVisibility.Public,
				IAppVisibility.Prototype,
				IAppVisibility.PublicRequestAccess,
				IAppVisibility.Private,
			],
			group: groups.collaborate,
			devOnly: true,
		},
		{
			href: "/library/config/roles",
			label: t("roles", "Roles"),
			icon: CrownIcon,
			description: t(
				"defineUserRolesAndAccessLevels",
				"Define user roles and access levels",
			),
			visibilities: [
				IAppVisibility.Public,
				IAppVisibility.Prototype,
				IAppVisibility.PublicRequestAccess,
			],
			group: groups.collaborate,
			devOnly: true,
		},
		{
			href: "/library/config/sales",
			label: t("sales", "Sales"),
			icon: DollarSignIcon,
			description: t(
				"trackSalesManagePricingAndDiscounts",
				"Track sales, manage pricing and discounts",
			),
			visibilities: [IAppVisibility.Public, IAppVisibility.PublicRequestAccess],
			requiresPaid: true,
			group: groups.insights,
			devOnly: true,
		},
		{
			href: "/library/config/analytics",
			label: t("analytics", "Analytics"),
			icon: ChartAreaIcon,
			description: t(
				"performanceMetricsAndInsights",
				"Performance metrics and insights",
			),
			group: groups.insights,
			devOnly: true,
		},
		{
			href: "/library/config/endpoints",
			label: t("endpoints", "Endpoints"),
			icon: GlobeIcon,
			description: t(
				"apiEndpointsAndIntegrations",
				"API endpoints and integrations",
			),
			group: groups.insights,
			devOnly: true,
		},
		{
			href: "/library/config/publication",
			label: t("publication", "Publication"),
			icon: SendIcon,
			description: t(
				"trackPublicationReviewStatusAndAuditorFeedback",
				"Track publication review status and auditor feedback",
			),
			group: groups.insights,
			devOnly: true,
		},
	];
}

function isRouteActive(itemHref: string, currentRoute: string): boolean {
	if (itemHref === "/library/config") {
		return currentRoute === "/library/config";
	}
	return currentRoute.startsWith(itemHref);
}

export default function Id({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const executionService = useExecutionServiceOptional();
	const searchParams = useSearchParams();
	const id = searchParams.get("id");
	const online = useLiveQuery(
		() =>
			appsDB.visibility
				.where("appId")
				.equals(id ?? "")
				.first(),
		[id ?? ""],
	) ?? { visibility: IAppVisibility.Offline };
	const currentRoute = usePathname();
	// Storage browsers and Data Studio own their vertical space; every other
	// section scrolls the page.
	const contentFillsHeight = configRouteFillsHeight(currentRoute);
	const router = useRouter();
	const metadata = useInvoke(
		backend.appState.getAppMeta,
		backend.appState,
		[id ?? ""],
		typeof id === "string",
	);
	const app = useInvoke(
		backend.appState.getApp,
		backend.appState,
		[id ?? ""],
		typeof id === "string",
	);

	const [isMaximized, setIsMaximized] = useState(false);
	const [exportOpen, setExportOpen] = useState(false);
	const [encrypt, setEncrypt] = useState(false);
	const [password, setPassword] = useState("");
	const [confirmPassword, setConfirmPassword] = useState("");
	const [showPassword, setShowPassword] = useState(false);
	const [exporting, setExporting] = useState(false);
	const [mobileNavOpen, setMobileNavOpen] = useState(false);
	const [mobileNavFilter, setMobileNavFilter] = useState("");
	const [lockedItem, setLockedItem] = useState<INavigationItem | null>(null);
	const touchStartRef = useRef<{ x: number; y: number } | null>(null);

	const settingsProfile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const publicationRequests = useQuery<
		RawAppPublicationRequestItem[],
		Error,
		AppPublicationRequestItem[]
	>({
		queryKey: ["app-publication-requests", id],
		queryFn: async () => {
			if (!settingsProfile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<RawAppPublicationRequestItem[]>(
				settingsProfile.data.hub_profile,
				`apps/${id}/publication`,
			);
		},
		enabled: !!settingsProfile.data && !!id,
		select: normalizeAppPublicationRequests,
	});

	const hasActivePublicationRequest = useMemo(
		() =>
			(publicationRequests.data ?? []).some(
				(r) => r.status === "pending" || r.status === "on_hold",
			),
		[publicationRequests.data],
	);

	const { developerMode } = useDeveloperMode();

	const visibility = online?.visibility ?? IAppVisibility.Offline;

	// Nav items visible for this app's visibility + paywall state — shared by the
	// desktop sidebar and the mobile bottom-sheet switcher (no double filtering).
	// Items whose visibility gate can be lifted stay in the list as `locked`.
	const visibleNavItems = useMemo(
		() =>
			buildNavigationItems(t)
				.filter(
					(item) =>
						(!item.devOnly || developerMode) &&
						(!item.visibilities ||
							item.visibilities.includes(visibility) ||
							item.lockedVisibilities?.includes(visibility)) &&
						(!item.requiresPaid ||
							(app.data?.price != null && app.data.price > 0)),
				)
				.map((item) => ({
					...item,
					locked:
						!!item.visibilities && !item.visibilities.includes(visibility),
				})),
		[visibility, app.data?.price, developerMode, t],
	);

	const activeItem = useMemo(
		() =>
			visibleNavItems.find((item) => isRouteActive(item.href, currentRoute)),
		[visibleNavItems, currentRoute],
	);

	const openLockedItem = useCallback((item: INavigationItem) => {
		setMobileNavOpen(false);
		setMobileNavFilter("");
		setLockedItem(item);
	}, []);

	// The nav reads the locally cached visibility, so mirror the new value right
	// away instead of waiting for the background app refetch to land, then take
	// the user to the section they originally clicked.
	const unlockSection = useCallback(
		async (item: INavigationItem, next: IAppVisibility) => {
			if (!id) return;
			await appsDB.visibility.put({ appId: id, visibility: next });
			router.push(`${item.href}?id=${id}`);
		},
		[id, router],
	);

	const [isIosTauri, setIsIosTauri] = useState(false);
	useEffect(() => {
		setIsIosTauri(isIosTauriRuntime());
	}, []);

	// Lock page scroll on desktop (md+) so only the right card scrolls
	useEffect(() => {
		if (typeof window === "undefined") return;
		const apply = () => {
			const isDesktop = window.matchMedia("(min-width: 768px)").matches;
			// Only lock on desktop; keep mobile natural scrolling
			document.body.style.overflowY = isDesktop ? "hidden" : "";
			document.documentElement.style.overflowY = isDesktop ? "hidden" : "";
		};
		apply();
		window.addEventListener("resize", apply);
		return () => {
			window.removeEventListener("resize", apply);
			document.body.style.overflowY = "";
			document.documentElement.style.overflowY = "";
		};
	}, []);

	useEffect(() => {
		const saved =
			typeof window !== "undefined"
				? localStorage.getItem("exportEncrypted")
				: null;
		if (saved != null) setEncrypt(saved === "true");
	}, []);

	useEffect(() => {
		if (typeof window !== "undefined")
			localStorage.setItem("exportEncrypted", String(encrypt));
		if (!encrypt) {
			setPassword("");
			setConfirmPassword("");
		}
	}, [encrypt]);

	useEffect(() => {
		if (!isIosTauri) return;

		const onTouchStart = (event: TouchEvent) => {
			const t = event.changedTouches[0];
			if (!t) return;
			touchStartRef.current = { x: t.clientX, y: t.clientY };
		};

		const onTouchEnd = (event: TouchEvent) => {
			const isMobileViewport = window.matchMedia("(max-width: 767px)").matches;
			if (!isMobileViewport || mobileNavOpen) return;

			const start = touchStartRef.current;
			const t = event.changedTouches[0];
			if (!start || !t) return;

			const dx = t.clientX - start.x;
			const dy = Math.abs(t.clientY - start.y);

			// Right-edge swipe left opens the config menu on iOS.
			if (start.x >= window.innerWidth - 24 && dx < -40 && dy < 30) {
				setMobileNavOpen(true);
			}

			touchStartRef.current = null;
		};

		window.addEventListener("touchstart", onTouchStart, { passive: true });
		window.addEventListener("touchend", onTouchEnd, { passive: true });

		return () => {
			window.removeEventListener("touchstart", onTouchStart);
			window.removeEventListener("touchend", onTouchEnd);
		};
	}, [isIosTauri, mobileNavOpen]);

	const events = useInvoke(
		backend.eventState.getEvents,
		backend.eventState,
		[id ?? ""],
		(id ?? "") !== "",
	);

	// Fetch configured routes for this app
	const routes = useInvoke(
		backend.routeState.getRoutes,
		backend.routeState,
		[id ?? ""],
		(id ?? "") !== "",
	);

	const { update } = useMobileHeader();

	const usableEvents = useMemo(() => {
		const set = new Set<string>();
		Object.values(EVENT_CONFIG).forEach((config) => {
			const usable = Object.keys(config.useInterfaces);
			for (const eventType of usable) {
				if (config.eventTypes.includes(eventType)) set.add(eventType);
			}
		});
		return set;
	}, []);

	const useAppHref = useMemo(() => {
		if (!id) return null;

		const activeEvents = (events.data ?? []).filter((event) => event.active);
		const activeEventsById = new Map(
			activeEvents.map((event) => [event.id, event] as const),
		);

		const hasUsableRoute = (routes.data ?? []).some((route) => {
			const routeEvent = activeEventsById.get(route.eventId);
			if (!routeEvent) return false;
			return (
				!!routeEvent.default_page_id || usableEvents.has(routeEvent.event_type)
			);
		});

		if (hasUsableRoute) {
			return `/use?id=${id}`;
		}

		const fallbackEvent = activeEvents.find((event) =>
			usableEvents.has(event.event_type),
		);
		if (!fallbackEvent) return null;

		return `/use?id=${id}&eventId=${fallbackEvent.id}`;
	}, [id, events.data, routes.data, usableEvents]);

	useEffect(() => {
		const canUseApp = !!useAppHref;
		const appName =
			metadata.data?.name ??
			(metadata.isFetching ? <Skeleton className="h-4 w-24" /> : "Unknown App");
		// Inside a sub-section the header shows "where you are" + a back chevron to
		// the app dashboard; on the dashboard it just shows the app name.
		const inSubSection = !!activeItem && activeItem.href !== "/library/config";

		update({
			title: activeItem?.label ?? appName,
			left: [
				<Button
					key="config-menu"
					variant="outline"
					size="icon"
					className="md:hidden size-9"
					onClick={() => setMobileNavOpen(true)}
					aria-label={t("configurationSections", "Configuration sections")}
				>
					<MenuIcon className="w-4 h-4" />
				</Button>,
				inSubSection ? (
					<Link
						key="config-back"
						href={`/library/config?id=${id}`}
						className="md:hidden"
						aria-label={t("backToDashboard", "Back to dashboard")}
					>
						<Button variant="ghost" size="icon" className="size-9">
							<ChevronLeftIcon className="w-5 h-5" />
						</Button>
					</Link>
				) : null,
			],
			right: canUseApp ? (
				<Link key="use-app" href={useAppHref} className="md:hidden">
					<Button
						variant="default"
						size="sm"
						aria-label={t("useApp", "Use App")}
					>
						<SparklesIcon className="w-4 h-4" />
						{t("useApp", "Use App")}
					</Button>
				</Link>
			) : undefined,
		});
	}, [
		metadata.data?.name,
		metadata.isFetching,
		useAppHref,
		id,
		update,
		activeItem,
	]);

	const strength = useMemo(() => {
		if (!encrypt) return 0;
		let s = 0;
		if (password.length >= 8) s++;
		if (/[A-Z]/.test(password) && /[a-z]/.test(password)) s++;
		if (/\d/.test(password)) s++;
		if (/[^A-Za-z0-9]/.test(password)) s++;
		return s;
	}, [password, encrypt]);

	const passValid =
		!encrypt || (password.length >= 8 && password === confirmPassword);

	const handleExport = useCallback(async () => {
		const loader = toast.loading(t("exportingApp", "Exporting app..."), {
			description: t(
				"thisMayTakeAMomentPleaseWait",
				"This may take a moment, please wait.",
			),
		});
		setExporting(true);
		try {
			await invoke("export_app_to_file", {
				appId: id,
				...(encrypt && password ? { password } : {}),
			});
			toast.success(
				t("appExportedSuccessfully", "App exported successfully!"),
				{ id: loader },
			);
			setExportOpen(false);
			setPassword("");
			setConfirmPassword("");
		} catch (error) {
			console.error("Export error:", error);
			toast.error("Failed to export app");
		} finally {
			setExporting(false);
			toast.dismiss(loader);
		}
	}, [id, encrypt, password]);

	async function executeEvent(event: IEvent) {
		if (!id) return;
		const runMeta = executionService
			? await executionService.executeEvent(
					id,
					event.id,
					{ id: event.node_id },
					false,
					() => {},
					() => {},
				)
			: await backend.eventState.executeEvent(
					id,
					event.id,
					{ id: event.node_id },
					false,
					() => {},
					() => {},
				);
		if (!runMeta) {
			toastError(
				t("failedToExecuteBoard", "Failed to execute board"),
				<PlayCircleIcon className="w-4 h-4" />,
			);
		}
	}

	return (
		<TooltipProvider>
			<main className="flex overflow-hidden flex-col w-full p-4 sm:p-6 gap-4 sm:gap-6 flex-1 min-h-0 h-full">
				{!isMaximized && (
					<Card className="border-0 shadow-sm bg-linear-to-r from-background to-muted/20 h-fit py-3 sm:py-4 hidden md:flex">
						<CardContent className="p-4 py-0 flex flex-row items-center justify-between">
							<Breadcrumb>
								<BreadcrumbList>
									<BreadcrumbItem>
										<BreadcrumbLink
											href="/library"
											className="flex items-center gap-1"
										>
											<LayoutGridIcon className="w-3 h-3" />
											{t("home", "Home")}
										</BreadcrumbLink>
									</BreadcrumbItem>
									<BreadcrumbSeparator />
									<BreadcrumbItem>
										<BreadcrumbPage className="font-medium flex flex-row items-center gap-2">
											{metadata.isFetching ? (
												<Skeleton className="h-4 w-24" />
											) : (
												metadata.data?.name
											)}
											{app.data?.visibility && (
												<div className="bg-gray-600/40 dark:bg-background rounded-full">
													<VisibilityIcon visibility={app.data?.visibility} />
												</div>
											)}
										</BreadcrumbPage>
									</BreadcrumbItem>
								</BreadcrumbList>
							</Breadcrumb>
							<div className="flex items-center gap-2">
								<div className="hidden md:block">
									{useAppHref ? (
										<Link href={useAppHref} className="w-full">
											<Button
												size="sm"
												className="flex items-center gap-2 w-full rounded-full px-4"
											>
												<SparklesIcon className="w-4 h-4" />
												<h4 className="text-sm font-medium">
													{t("useApp", "Use App")}
												</h4>
											</Button>
										</Link>
									) : (
										<Link href={`/library/config?id=${id}`} className="w-full">
											<Button
												size="sm"
												className="flex items-center gap-2 w-full rounded-full px-4"
											>
												<WorkflowIcon className="w-4 h-4" />
												<h4 className="text-sm font-medium">
													{t("startBuilding", "Start Building")}
												</h4>
											</Button>
										</Link>
									)}
								</div>

								{/* Mobile quick action */}
								{useAppHref ? (
									<Link href={useAppHref} className="md:hidden">
										<Button
											variant="default"
											size="sm"
											aria-label={t("useApp", "Use App")}
										>
											<SparklesIcon className="w-4 h-4" />
											{t("useApp", "Use App")}
										</Button>
									</Link>
								) : (
									<Link href={`/library/config?id=${id}`} className="md:hidden">
										<Button
											variant="default"
											size="sm"
											aria-label={t("startBuilding", "Start Building")}
										>
											<WorkflowIcon className="w-4 h-4" />
											{t("startBuilding", "Start Building")}
										</Button>
									</Link>
								)}

								<Button
									variant="outline"
									size="sm"
									className="md:hidden"
									onClick={() => setMobileNavOpen(true)}
									aria-label={t("openMenu", "Open menu")}
								>
									<MenuIcon className="w-4 h-4" />
								</Button>
							</div>
						</CardContent>
					</Card>
				)}

				{/* Mobile configuration-section switcher: a grouped, searchable
				    bottom sheet with an active highlight and thumb-sized rows. */}
				<Sheet
					open={mobileNavOpen}
					onOpenChange={(open) => {
						setMobileNavOpen(open);
						if (!open) setMobileNavFilter("");
					}}
				>
					<SheetContent
						side="bottom"
						className="h-[85dvh] max-h-[85dvh] overflow-hidden p-0 flex flex-col rounded-t-2xl pb-safe"
					>
						<SheetHeader className="p-4 pb-3 border-b text-left space-y-3">
							<div>
								<SheetTitle>
									{t("configure", "Configure")} {metadata.data?.name ?? "app"}
								</SheetTitle>
								<SheetDescription>
									{t(
										"jumpToAConfigurationSection",
										"Jump to a configuration section",
									)}
								</SheetDescription>
							</div>
							<div className="relative">
								<SearchIcon className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 w-4 h-4 text-muted-foreground" />
								<Input
									value={mobileNavFilter}
									onChange={(e) => setMobileNavFilter(e.target.value)}
									placeholder={t("filterSections", "Filter sections…")}
									className="pl-8 h-10"
									aria-label={t(
										"filterConfigurationSections",
										"Filter configuration sections",
									)}
								/>
							</div>
						</SheetHeader>
						<div className="flex-1 min-h-0 overflow-y-auto overscroll-contain touch-pan-y p-3 [-webkit-overflow-scrolling:touch]">
							<nav
								className="flex flex-col gap-0.5"
								key={id + (online?.visibility ?? "")}
							>
								{(() => {
									const query = mobileNavFilter.trim().toLowerCase();
									const filtered = query
										? visibleNavItems.filter(
												(item) =>
													item.label.toLowerCase().includes(query) ||
													item.group.toLowerCase().includes(query) ||
													item.description.toLowerCase().includes(query),
											)
										: visibleNavItems;
									if (filtered.length === 0) {
										return (
											<p className="px-3 py-8 text-center text-sm text-muted-foreground">
												{t(
													"noSectionsMatchMobilenavfilter",
													"No sections match “{{mobileNavFilter}}”.",
													{ mobileNavFilter },
												)}
											</p>
										);
									}
									let lastGroup = "";
									return filtered.map((item) => {
										const Icon = item.icon;
										const showGroupHeader = item.group !== lastGroup;
										lastGroup = item.group;
										const active = isRouteActive(item.href, currentRoute);
										return (
											<div key={item.href}>
												{showGroupHeader && (
													<div className="text-[11px] font-semibold text-muted-foreground/70 uppercase tracking-wider px-3 pt-4 pb-1 first:pt-0">
														{item.group}
													</div>
												)}
												{item.disabled ? (
													<div
														className="flex items-center gap-3 px-3 min-h-11 rounded-lg text-sm text-muted-foreground bg-muted/50 opacity-60"
														aria-disabled="true"
													>
														<Icon className="w-4 h-4 shrink-0" />
														<span className="truncate">
															{t("labelSoon", "{{label}} (soon)", {
																label: item.label,
															})}
														</span>
													</div>
												) : item.locked ? (
													<button
														type="button"
														className="w-full flex items-center gap-3 px-3 min-h-11 rounded-lg text-sm text-muted-foreground/60 bg-muted/40 transition-colors"
														onClick={() => openLockedItem(item)}
													>
														<Icon className="w-4 h-4 shrink-0" />
														<span className="truncate">{item.label}</span>
														<LockIcon className="ml-auto w-3.5 h-3.5 shrink-0" />
													</button>
												) : (
													<Link
														href={`${item.href}?id=${id}`}
														className={`flex items-center gap-3 px-3 min-h-11 rounded-lg text-sm transition-colors ${
															active
																? "bg-primary/10 text-primary font-medium"
																: "text-muted-foreground hover:bg-muted hover:text-foreground"
														}`}
														onClick={() => {
															setMobileNavOpen(false);
															setMobileNavFilter("");
														}}
													>
														<Icon className="w-4 h-4 shrink-0" />
														<span className="truncate">{item.label}</span>
														{item.href === "/library/config/publication" &&
															hasActivePublicationRequest && (
																<span className="ml-auto w-2 h-2 rounded-full bg-blue-500 shrink-0" />
															)}
													</Link>
												)}
											</div>
										);
									});
								})()}

								{(online?.visibility ?? IAppVisibility.Private) ===
									IAppVisibility.Offline && (
									<Button
										variant="ghost"
										className="flex items-center gap-3 px-3 min-h-11 justify-start text-foreground mt-1"
										onClick={() => {
											setMobileNavOpen(false);
											setMobileNavFilter("");
											setExportOpen(true);
										}}
									>
										<DownloadIcon className="w-4 h-4 shrink-0" />
										<span className="truncate">
											{t("exportApp", "Export App")}
										</span>
									</Button>
								)}
							</nav>
						</div>
					</SheetContent>
				</Sheet>

				{/* Unlock dialog for nav sections gated behind a visibility change */}
				{id && lockedItem && (
					<VisibilityUpgradeDialog
						appId={id}
						open
						onOpenChange={(open) => {
							if (!open) setLockedItem(null);
						}}
						feature={lockedItem.label}
						reason={lockedItem.lockedReason ?? lockedItem.description}
						current={visibility}
						target={lockedItem.unlockVisibility ?? IAppVisibility.Prototype}
						onChanged={(next) => unlockSection(lockedItem, next)}
					/>
				)}

				{/* Global Export Dialog */}
				<Dialog open={exportOpen} onOpenChange={setExportOpen}>
					<DialogContent className="sm:max-w-[520px]">
						<DialogHeader>
							<DialogTitle>
								{t("exportApplication", "Export Application")}
							</DialogTitle>
							<DialogDescription>
								{`Choose how you want to export your app.`}
							</DialogDescription>
						</DialogHeader>

						<div className="space-y-4">
							<div className="flex items-center justify-between rounded-lg border p-3">
								<div className="flex items-center gap-3">
									{encrypt ? (
										<LockIcon className="w-4 h-4 text-primary" />
									) : (
										<UnlockIcon className="w-4 h-4 text-muted-foreground" />
									)}
									<div className="min-w-0">
										<p className="text-sm font-medium">
											{encrypt ? "Encrypted export" : "Unencrypted export"}
										</p>
										<p className="text-xs text-muted-foreground">
											{encrypt
												? `Protect your export with a password.`
												: `Quick export without encryption.`}
										</p>
									</div>
								</div>
								<div className="flex items-center gap-2">
									<span className="text-xs text-muted-foreground">
										{t("encrypt", "Encrypt")}
									</span>
									<Switch checked={encrypt} onCheckedChange={setEncrypt} />
								</div>
							</div>

							{encrypt && (
								<div className="space-y-3">
									<div className="grid gap-2">
										<Label htmlFor="export-password" className="text-xs">
											{t("password", "Password")}
										</Label>
										<div className="relative">
											<Input
												id="export-password"
												type={showPassword ? "text" : "password"}
												value={password}
												onChange={(e) => setPassword(e.target.value)}
												placeholder={t(
													"enterAStrongPassword",
													"Enter a strong password",
												)}
												autoFocus
											/>
											<Button
												type="button"
												variant="ghost"
												size="icon"
												className="absolute right-1 top-1 h-7 w-7"
												onClick={() => setShowPassword((s) => !s)}
												aria-label={
													showPassword ? "Hide password" : "Show password"
												}
											>
												{showPassword ? (
													<EyeOffIcon className="w-4 h-4" />
												) : (
													<EyeIcon className="w-4 h-4" />
												)}
											</Button>
										</div>
									</div>

									<div className="grid gap-2">
										<Label
											htmlFor="export-password-confirm"
											className="text-xs"
										>
											{t("confirmPassword", "Confirm password")}
										</Label>
										<Input
											id="export-password-confirm"
											type={showPassword ? "text" : "password"}
											value={confirmPassword}
											onChange={(e) => setConfirmPassword(e.target.value)}
											placeholder={t("reenterPassword", "Re-enter password")}
										/>
									</div>

									<div className="flex items-center gap-2">
										<div className="flex gap-1" aria-hidden>
											{[0, 1, 2, 3].map((i) => (
												<span
													key={i}
													className={`h-1.5 w-10 rounded ${strength > i ? "bg-green-500" : "bg-muted"}`}
												/>
											))}
										</div>
										<span className="text-xs text-muted-foreground">
											{strength <= 1
												? "Weak"
												: strength === 2
													? "Fair"
													: strength === 3
														? "Good"
														: "Strong"}
										</span>
									</div>

									{!passValid && (
										<p className="text-xs text-destructive">
											{t(
												"passwordsMustMatchAndBeAtLeast8Characters",
												"Passwords must match and be at least 8 characters.",
											)}
										</p>
									)}
								</div>
							)}
						</div>

						<DialogFooter className="gap-2">
							<Button
								variant="outline"
								onClick={() => setExportOpen(false)}
								disabled={exporting}
							>
								{t("cancel", "Cancel")}
							</Button>
							<Button
								onClick={handleExport}
								disabled={exporting || (encrypt && !passValid)}
							>
								{exporting ? "Exporting..." : "Export"}
							</Button>
						</DialogFooter>
					</DialogContent>
				</Dialog>

				<div
					className={`grid w-full items-stretch gap-6 flex-1 overflow-hidden min-h-0 transition-all duration-300 ${isMaximized ? "grid-cols-1" : "md:grid-cols-[240px_1fr] lg:grid-cols-[260px_1fr]"}`}
				>
					{!isMaximized && (
						<Card className="h-full max-h-full overflow-hidden py-2 hidden md:flex md:flex-col md:grow order-2 md:order-1">
							<CardContent className="flex-1 p-0 overflow-hidden">
								<ScrollArea className="h-full px-3 flex-1">
									<nav
										className="flex flex-col gap-0.5 py-3"
										key={id + (online?.visibility ?? "")}
									>
										{(() => {
											const filtered = visibleNavItems;
											let lastGroup = "";
											return filtered.map((item) => {
												const Icon = item.icon;
												const showGroupHeader = item.group !== lastGroup;
												lastGroup = item.group;
												const active = isRouteActive(item.href, currentRoute);
												return (
													<div key={item.href}>
														{showGroupHeader && (
															<div className="text-[11px] font-semibold text-muted-foreground/70 uppercase tracking-wider px-3 pt-4 pb-1 first:pt-0">
																{item.group}
															</div>
														)}
														{item.disabled ? (
															<Tooltip delayDuration={300}>
																<TooltipTrigger asChild>
																	<div
																		className="flex items-center gap-3 px-3 py-2 rounded-lg text-sm text-muted-foreground bg-muted/50 opacity-60 cursor-not-allowed"
																		tabIndex={-1}
																		aria-disabled="true"
																	>
																		<Icon className="w-4 h-4 shrink-0" />
																		<span className="truncate">
																			{item.label}
																		</span>
																	</div>
																</TooltipTrigger>
																<TooltipContent
																	side="right"
																	className="max-w-xs"
																>
																	<p className="font-bold">
																		{t(
																			"labelComingSoon",
																			"{{label}} (Coming soon!)",
																			{ label: item.label },
																		)}
																	</p>
																	<p className="text-xs mt-1">
																		{item.description}
																	</p>
																</TooltipContent>
															</Tooltip>
														) : item.locked ? (
															<Tooltip delayDuration={300}>
																<TooltipTrigger asChild>
																	<button
																		type="button"
																		className="w-full flex items-center gap-3 px-3 py-2 rounded-lg text-sm text-muted-foreground/60 bg-muted/40 hover:bg-muted hover:text-muted-foreground transition-all"
																		onClick={() => openLockedItem(item)}
																	>
																		<Icon className="w-4 h-4 shrink-0" />
																		<span className="truncate">
																			{item.label}
																		</span>
																		<LockIcon className="ml-auto w-3.5 h-3.5 shrink-0" />
																	</button>
																</TooltipTrigger>
																<TooltipContent
																	side="right"
																	className="max-w-xs"
																>
																	<p className="font-bold">
																		{t("labelLocked", "{{label}} (locked)", {
																			label: item.label,
																		})}
																	</p>
																	<p className="text-xs mt-1">
																		{item.lockedReason ?? item.description}
																	</p>
																</TooltipContent>
															</Tooltip>
														) : (
															<Tooltip delayDuration={300}>
																<TooltipTrigger asChild>
																	<Link
																		href={`${item.href}?id=${id}`}
																		className={`flex items-center gap-3 px-3 py-2 rounded-lg text-sm transition-all ${
																			active
																				? "bg-primary/10 text-primary font-medium"
																				: "text-muted-foreground hover:bg-muted hover:text-foreground"
																		}`}
																	>
																		<Icon className="w-4 h-4 shrink-0" />
																		<span className="truncate">
																			{item.label}
																		</span>
																		{item.href ===
																			"/library/config/publication" &&
																			hasActivePublicationRequest && (
																				<span className="ml-auto w-2 h-2 rounded-full bg-blue-500 shrink-0" />
																			)}
																	</Link>
																</TooltipTrigger>
																<TooltipContent
																	side="right"
																	className="max-w-xs"
																>
																	<p className="font-bold">{item.label}</p>
																	<p className="text-xs mt-1">
																		{item.description}
																	</p>
																</TooltipContent>
															</Tooltip>
														)}
													</div>
												);
											});
										})()}
										{(online?.visibility ?? IAppVisibility.Private) ===
											IAppVisibility.Offline && (
											<Tooltip key="export" delayDuration={300}>
												<TooltipTrigger asChild>
													<Button
														variant="link"
														className="flex items-center gap-3 px-3 py-2 rounded-lg text-sm transition-all justify-start hover:bg-muted text-muted-foreground hover:text-foreground"
														onClick={() => setExportOpen(true)}
													>
														<DownloadIcon className="w-4 h-4 shrink-0" />
														<span className="truncate">
															{t("exportApp", "Export App")}
														</span>
													</Button>
												</TooltipTrigger>
												<TooltipContent side="right" className="max-w-xs">
													<p className="font-bold">
														{t("exportApplication", "Export Application")}
													</p>
													<p className="text-xs mt-1">
														{t(
															"exportTheApplicationToAFileForBackupOrSharing",
															"Export the application to a file for backup or sharing.",
														)}
													</p>
												</TooltipContent>
											</Tooltip>
										)}
									</nav>

									<Separator className="my-4 mx-3" />

									<div className="px-3">
										<div className="flex items-center gap-2 mb-3">
											<ZapIcon className="w-4 h-4 text-primary" />
											<h4 className="text-sm font-medium">
												{t("quickActions", "Quick Actions")}
											</h4>
										</div>
										<div className="flex flex-col gap-2 pb-4">
											{events.data &&
											events.data.filter(
												(e) => e.event_type === "quick_action" && e.active,
											).length > 0 ? (
												events.data
													.filter(
														(e) => e.event_type === "quick_action" && e.active,
													)
													.map((event) => (
														<HoverCard
															key={event.id}
															openDelay={100}
															closeDelay={100}
														>
															<HoverCardTrigger asChild>
																<Button
																	variant="outline"
																	size="sm"
																	className="justify-start gap-2 h-auto py-2 px-3"
																	onClick={async () => {
																		await executeEvent(event);
																	}}
																>
																	<PlayCircleIcon className="w-3 h-3 text-green-600" />
																	<span className="truncate text-xs">
																		{event.name}
																	</span>
																</Button>
															</HoverCardTrigger>
															<HoverCardContent side="right" className="w-80">
																<div className="space-y-2">
																	<div>
																		<h4 className="text-base font-medium">
																			{event.name}
																		</h4>
																		<p className="text-sm text-muted-foreground">
																			{event.description}
																		</p>
																	</div>
																</div>
															</HoverCardContent>
														</HoverCard>
													))
											) : (
												<p className="text-xs text-muted-foreground py-2">
													{t(
														"noQuickActionsAvailable",
														"No quick actions available",
													)}
												</p>
											)}
										</div>
									</div>
								</ScrollArea>
							</CardContent>
						</Card>
					)}

					<Card
						className={`relative h-full max-h-full flex-col grow overflow-hidden min-h-0 transition-all duration-300 bg-transparent flex border-0 shadow-none md:border md:shadow-sm ${isMaximized ? "shadow-2xl" : ""} order-1 md:order-2`}
					>
						<div className="pointer-events-none absolute right-4 top-4 z-20 hidden md:block">
							<div className="pointer-events-auto">
								<Tooltip>
									<TooltipTrigger asChild>
										<Button
											variant="ghost"
											size="sm"
											onClick={() => setIsMaximized(!isMaximized)}
											className="h-8 w-8 p-0 bg-background/80 backdrop-blur-sm"
										>
											{isMaximized ? (
												<Minimize2Icon className="w-4 h-4" />
											) : (
												<Maximize2Icon className="w-4 h-4" />
											)}
										</Button>
									</TooltipTrigger>
									<TooltipContent>
										{isMaximized ? "Minimize" : "Maximize"}
									</TooltipContent>
								</Tooltip>
							</div>
						</div>
						<CardContent className="flex-1 p-0 overflow-hidden min-h-0">
							{hasActivePublicationRequest &&
								!currentRoute?.includes("/publication") && (
									<div className="px-0 pt-4 md:px-6 md:pr-16">
										<AppPublicationBanner
											requests={publicationRequests.data ?? []}
											onNavigate={() => {
												window.location.href = `/library/config/publication?id=${id}`;
											}}
										/>
									</div>
								)}
							<div
								className={
									contentFillsHeight
										? "h-full flex flex-col"
										: "h-full overflow-y-auto"
								}
							>
								<div
									className={
										contentFillsHeight
											? "flex-1 min-h-0 overflow-hidden p-0 md:p-6 md:pt-4 md:pr-16"
											: "p-0 md:p-6 md:pt-4 md:pr-16"
									}
								>
									<Suspense
										fallback={
											<div className="space-y-4">
												<Skeleton className="h-8 w-full" />
												<Skeleton className="h-32 w-full" />
												<Skeleton className="h-24 w-full" />
											</div>
										}
									>
										<div key={id ?? "missing-app"} className="contents">
											{children}
										</div>
									</Suspense>
								</div>
							</div>
						</CardContent>
					</Card>
				</div>
			</main>
		</TooltipProvider>
	);
}
