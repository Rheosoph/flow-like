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
	HoverCard,
	HoverCardContent,
	HoverCardTrigger,
	IAppVisibility,
	type IEvent,
	Input,
	RolePermissions,
	ScrollArea,
	Separator,
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
	Skeleton,
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
	VisibilityIcon,
	toastError,
	useAppPermissions,
	useBackend,
	useDeveloperMode,
	useExecutionServiceOptional,
	useInvoke,
	useMobileHeader,
} from "@flow-like/flow-like-ui";
import {
	ConfigNavRow,
	PermissionLockDialog,
	SectionLockedPanel,
} from "@flow-like/flow-like-ui/components/settings/permission";
import { AppPublicationBanner } from "@flow-like/flow-like-ui/components/settings/visibility-status/app-publication-banner";
import {
	type AppPublicationRequestItem,
	type RawAppPublicationRequestItem,
	normalizeAppPublicationRequests,
} from "@flow-like/flow-like-ui/components/settings/visibility-status/app-publication-review-card";
import { VisibilityUpgradeDialog } from "@flow-like/flow-like-ui/components/settings/visibility-status/visibility-upgrade-dialog";
import {
	type INavigationItem,
	type INavigationItemState,
	buildNavigationItems,
	isConfigRouteActive,
	resolveNavigationItems,
} from "@flow-like/flow-like-ui/lib/config-nav";
import { configRouteFillsHeight } from "@flow-like/flow-like-ui/lib/config-route";
import { EVENT_CONFIG } from "@flow-like/flow-like-ui/lib/event-config";
import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { useLiveQuery } from "dexie-react-hooks";
import {
	ChevronLeftIcon,
	DownloadIcon,
	LayoutGridIcon,
	Maximize2Icon,
	MenuIcon,
	Minimize2Icon,
	PlayCircleIcon,
	SearchIcon,
	SparklesIcon,
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
import { appsDB } from "../../../lib/apps-db";
import { isIosTauriRuntime } from "../../../lib/platform";
import ExportAppDialog from "../components/ExportAppDialog";

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
	const [mobileNavOpen, setMobileNavOpen] = useState(false);
	const [mobileNavFilter, setMobileNavFilter] = useState("");
	const [lockedItem, setLockedItem] = useState<INavigationItemState | null>(
		null,
	);
	const touchStartRef = useRef<{ x: number; y: number } | null>(null);

	const permissions = useAppPermissions(id);

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
			// A local-only app has never been submitted, and the hub does not know it.
			if (id && (await backend.isLocalOnly?.(id))) return [];
			return backend.apiState.get<RawAppPublicationRequestItem[]>(
				settingsProfile.data.hub_profile,
				`apps/${id}/publication`,
			);
		},
		// Reading publication requests is Admin-only, and this query runs on every
		// route under /library/config — without the guard every ordinary member
		// pays a 403 for a badge they will never see.
		enabled:
			!!settingsProfile.data && !!id && permissions.can(RolePermissions.Admin),
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

	const permissionLockReason = useCallback(
		(item: INavigationItem) =>
			t(
				"yourRoleOnThisProjectCannotOpenLabelDescription",
				"Your role on this project cannot open {{label}}. {{description}}",
				{ label: item.label, description: item.description },
			),
		[t],
	);

	// Nav items visible for this app's visibility, paywall and role — shared by
	// the desktop sidebar and the mobile bottom-sheet switcher (no double
	// filtering). Items behind a gate stay in the list carrying a `lock`.
	const visibleNavItems = useMemo(
		() =>
			resolveNavigationItems(buildNavigationItems(t), {
				visibility,
				developerMode,
				isPaid: app.data?.price != null && app.data.price > 0,
				can: permissions.can,
				permissionLockReason,
			}),
		[
			visibility,
			app.data?.price,
			developerMode,
			t,
			permissions.can,
			permissionLockReason,
		],
	);

	const activeItem = useMemo(
		() =>
			visibleNavItems.find((item) =>
				isConfigRouteActive(item.href, currentRoute),
			),
		[visibleNavItems, currentRoute],
	);

	const openLockedItem = useCallback((item: INavigationItemState) => {
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

	// A section the nav locks must stay locked when reached another way — a
	// pasted URL, the spotlight palette, a deep link from Home. The nav already
	// resolved the gate for this route, so reuse its verdict here instead of
	// letting the page mount and 403 its way to an empty screen.
	const lockedSection = activeItem?.lock ? (
		<SectionLockedPanel
			feature={activeItem.label}
			kind={activeItem.lock.kind}
			description={activeItem.lock.reason}
			missing={
				activeItem.lock.kind === "permission" ? activeItem.lock.missing : []
			}
			roleName={permissions.roleName}
			action={
				activeItem.lock.kind === "visibility" ? (
					<Button onClick={() => openLockedItem(activeItem)}>
						{t("unlockLabel", "Unlock {{label}}", { label: activeItem.label })}
					</Button>
				) : undefined
			}
		/>
	) : null;

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
										const showGroupHeader = item.group !== lastGroup;
										lastGroup = item.group;
										return (
											<div key={item.href}>
												{showGroupHeader && (
													<div className="text-[11px] font-semibold text-muted-foreground/70 uppercase tracking-wider px-3 pt-4 pb-1 first:pt-0">
														{item.group}
													</div>
												)}
												<ConfigNavRow
													item={item}
													appId={id ?? ""}
													variant="mobile"
													active={isConfigRouteActive(item.href, currentRoute)}
													trailing={
														item.href === "/library/config/publication" &&
														hasActivePublicationRequest ? (
															<span className="ml-auto w-2 h-2 rounded-full bg-primary shrink-0" />
														) : undefined
													}
													onNavigate={() => {
														setMobileNavOpen(false);
														setMobileNavFilter("");
													}}
													onLockClick={openLockedItem}
												/>
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

				{/* A visibility lock is a door this account can open; a permission
				    lock belongs to someone else, so it only explains itself. */}
				{id && lockedItem?.lock?.kind === "visibility" && (
					<VisibilityUpgradeDialog
						appId={id}
						open
						onOpenChange={(open) => {
							if (!open) setLockedItem(null);
						}}
						feature={lockedItem.label}
						reason={lockedItem.lock.reason}
						current={visibility}
						target={lockedItem.lock.target}
						onChanged={(next) => unlockSection(lockedItem, next)}
					/>
				)}

				{lockedItem?.lock?.kind === "permission" && (
					<PermissionLockDialog
						open
						onOpenChange={(open) => {
							if (!open) setLockedItem(null);
						}}
						feature={lockedItem.label}
						reason={lockedItem.lock.reason}
						missing={lockedItem.lock.missing}
						roleName={permissions.roleName}
					/>
				)}

				<ExportAppDialog
					appId={id}
					open={exportOpen}
					onOpenChange={setExportOpen}
				/>

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
											let lastGroup = "";
											return visibleNavItems.map((item) => {
												const showGroupHeader = item.group !== lastGroup;
												lastGroup = item.group;
												return (
													<div key={item.href}>
														{showGroupHeader && (
															<div className="text-[11px] font-semibold text-muted-foreground/70 uppercase tracking-wider px-3 pt-4 pb-1 first:pt-0">
																{item.group}
															</div>
														)}
														<ConfigNavRow
															item={item}
															appId={id ?? ""}
															variant="sidebar"
															withTooltip
															active={isConfigRouteActive(
																item.href,
																currentRoute,
															)}
															trailing={
																item.href === "/library/config/publication" &&
																hasActivePublicationRequest ? (
																	<span className="ml-auto w-2 h-2 rounded-full bg-primary shrink-0" />
																) : undefined
															}
															onLockClick={openLockedItem}
														/>
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
											{lockedSection ?? children}
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
