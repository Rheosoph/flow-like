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
	RolePermissions,
	ScrollArea,
	Separator,
	Skeleton,
	Switch,
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
	VisibilityIcon,
	toastError,
	useAppPermissions,
	useBackend,
	useDeveloperMode,
	useExecutionService,
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
import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { useLiveQuery } from "dexie-react-hooks";
import {
	DownloadIcon,
	EyeIcon,
	EyeOffIcon,
	LayoutGridIcon,
	LockIcon,
	Maximize2Icon,
	MenuIcon,
	Minimize2Icon,
	PlayCircleIcon,
	SparklesIcon,
	UnlockIcon,
	ZapIcon,
} from "lucide-react";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import { appsDB } from "../../../lib/apps-db";
import { EVENT_CONFIG } from "../../../lib/event-config";

export default function Id({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const executionService = useExecutionService();
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
	const [lockedItem, setLockedItem] = useState<INavigationItemState | null>(
		null,
	);
	const { developerMode } = useDeveloperMode();

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

	// Use local visibility if available, otherwise fall back to app.data.visibility
	const effectiveVisibility = useMemo(
		() =>
			online?.visibility !== IAppVisibility.Offline
				? online?.visibility
				: (app.data?.visibility ?? IAppVisibility.Offline),
		[online?.visibility, app.data?.visibility],
	);

	const visibility = effectiveVisibility ?? IAppVisibility.Offline;

	const permissionLockReason = useCallback(
		(item: INavigationItem) =>
			t(
				"yourRoleOnThisProjectCannotOpenLabelDescription",
				"Your role on this project cannot open {{label}}. {{description}}",
				{ label: item.label, description: item.description },
			),
		[t],
	);

	// Nav items visible for this app's visibility, paywall, dev-mode and role —
	// shared by the sidebar card and the mobile nav dialog (no double filtering).
	// Items behind a gate stay in the list carrying a `lock`.
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
			developerMode,
			visibility,
			app.data?.price,
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

	const { update } = useMobileHeader(
		{
			title:
				metadata.data?.name ??
				(metadata.isFetching ? (
					<Skeleton className="h-4 w-24" />
				) : (
					"Unknown App"
				)),
			right: (
				<Button
					key={"open-menu"}
					variant="outline"
					size="sm"
					className="md:hidden"
					onClick={() => setMobileNavOpen(true)}
					aria-label={t("openMenu", "Open menu")}
				>
					<MenuIcon className="w-4 h-4" />
				</Button>
			),
		},
		[events.data],
	);

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

		update({
			title:
				metadata.data?.name ??
				(metadata.isFetching ? (
					<Skeleton className="h-4 w-24" />
				) : (
					"Unknown App"
				)),
			right: [
				<Button
					key={"open-menu"}
					variant="outline"
					size="sm"
					className="md:hidden"
					onClick={() => setMobileNavOpen(true)}
					aria-label={t("openMenu", "Open menu")}
				>
					<MenuIcon className="w-4 h-4" />
				</Button>,
				...(canUseApp
					? [
							<Link key={"use-app"} href={useAppHref} className="md:hidden">
								<Button
									variant="default"
									size="sm"
									aria-label={t("useApp", "Use App")}
								>
									<SparklesIcon className="w-4 h-4" />
									{t("useApp", "Use App")}
								</Button>
							</Link>,
						]
					: []),
			],
		});
	}, [metadata.data?.name, metadata.isFetching, useAppHref, update]);

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
		// Export not supported in web mode - apps are stored remotely
		toast.info(t("exportNotAvailable", "Export not available"), {
			description: `App export is only available in the desktop app. Your app is automatically synced to the cloud.`,
		});
	}, []);

	async function executeEventHandler(event: IEvent) {
		if (!id) return;
		const runMeta = await executionService.executeEvent(
			id,
			event.id,
			{ id: event.node_id },
			true,
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

	// Storage and Data Studio own their vertical space; every other section scrolls.
	const contentFillsHeight = configRouteFillsHeight(currentRoute);

	// Rendered exactly once — a second copy for mobile would mount the whole page
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

	// twice, duplicating its effects, URL writes and IndexedDB persistence.
	const pageContent = (
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
	);

	return (
		<TooltipProvider>
			<main className="flex overflow-hidden flex-col w-full p-4 sm:p-6 gap-4 sm:gap-6 flex-1 min-h-0 h-full">
				{!isMaximized && (
					<Card className="border-0 shadow-sm bg-gradient-to-r from-background to-muted/20 h-fit py-3 sm:py-4 hidden md:flex">
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
								{useAppHref && (
									<div className="hidden md:block">
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
									</div>
								)}

								{/* Mobile "Use App" quick action */}
								{useAppHref && (
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

				{/* Mobile navigation dialog */}
				<Dialog open={mobileNavOpen} onOpenChange={setMobileNavOpen}>
					<DialogContent className="sm:max-w-[480px] p-0 overflow-hidden">
						<div className="p-4 border-b">
							<DialogTitle>{t("navigation", "Navigation")}</DialogTitle>
							<DialogDescription>
								{t(
									"quicklyJumpToSettingsSections",
									"Quickly jump to settings sections",
								)}
							</DialogDescription>
						</div>
						<ScrollArea className="max-h-[70vh]">
							<nav
								className="flex flex-col gap-1 p-3"
								key={id + (effectiveVisibility ?? "")}
							>
								{visibleNavItems.map((item) => (
									<ConfigNavRow
										key={item.href}
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
										onNavigate={() => setMobileNavOpen(false)}
										onLockClick={openLockedItem}
									/>
								))}

								{(effectiveVisibility ?? IAppVisibility.Private) ===
									IAppVisibility.Offline && (
									<Button
										variant="ghost"
										className="flex items-center gap-3 px-3 py-2 justify-start text-foreground"
										onClick={() => {
											setMobileNavOpen(false);
											setExportOpen(true);
										}}
									>
										<DownloadIcon className="w-4 h-4 flex-shrink-0" />
										<span className="truncate">
											{t("exportApp", "Export App")}
										</span>
									</Button>
								)}
							</nav>
						</ScrollArea>
					</DialogContent>
				</Dialog>

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
						<Card className="h-full max-h-full overflow-hidden py-2 hidden md:flex md:flex-col md:flex-grow order-2 md:order-1">
							<CardContent className="flex-1 p-0 overflow-hidden">
								<ScrollArea className="h-full px-3 flex-1">
									<nav
										className="flex flex-col gap-0.5 py-3"
										key={id + (effectiveVisibility ?? "")}
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
										{(effectiveVisibility ?? IAppVisibility.Private) ===
											IAppVisibility.Offline && (
											<Tooltip key="export" delayDuration={300}>
												<TooltipTrigger asChild>
													<Button
														variant="link"
														className="flex items-center gap-3 px-3 py-2 rounded-lg text-sm transition-all justify-start hover:bg-muted text-muted-foreground hover:text-foreground"
														onClick={() => setExportOpen(true)}
													>
														<DownloadIcon className="w-4 h-4 flex-shrink-0" />
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
																		await executeEventHandler(event);
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
						className={`relative h-full max-h-full flex flex-col grow overflow-hidden min-h-0 transition-all duration-300 bg-transparent border-0 rounded-none py-0 shadow-none backdrop-blur-none md:border md:rounded-xl md:py-6 md:shadow-sm md:backdrop-blur-sm ${isMaximized ? "md:shadow-2xl" : ""} order-1 md:order-2`}
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
									<div className="px-3 pt-4 md:px-6 md:pr-16">
										<AppPublicationBanner
											requests={publicationRequests.data ?? []}
											onNavigate={() => {
												window.location.href = `/library/config/publication?id=${id}`;
											}}
										/>
									</div>
								)}
							{contentFillsHeight ? (
								<div className="h-full flex flex-col">
									<div className="flex-1 min-h-0 overflow-hidden px-3 pb-4 md:p-6 md:pt-4 md:pr-16">
										{pageContent}
									</div>
								</div>
							) : (
								<div className="h-full overflow-y-auto">
									<div className="px-3 pb-4 md:p-6 md:pt-4 md:pr-16">
										{pageContent}
									</div>
								</div>
							)}
						</CardContent>
					</Card>
				</div>
			</main>
		</TooltipProvider>
	);
}
