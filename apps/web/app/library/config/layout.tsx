"use client";

import { useQuery } from "@tanstack/react-query";
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
	Skeleton,
	Switch,
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
	VisibilityIcon,
	toastError,
	useBackend,
	useExecutionService,
	useInvoke,
	useMobileHeader,
} from "@flow-like/flow-like-ui";
import { AppPublicationBanner } from "@flow-like/flow-like-ui/components/settings/visibility-status/app-publication-banner";
import {
	type AppPublicationRequestItem,
	type RawAppPublicationRequestItem,
	normalizeAppPublicationRequests,
} from "@flow-like/flow-like-ui/components/settings/visibility-status/app-publication-review-card";
import { useLiveQuery } from "dexie-react-hooks";
import {
	ChartAreaIcon,
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
import { usePathname, useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";
import { appsDB } from "../../../lib/apps-db";
import { EVENT_CONFIG } from "../../../lib/event-config";

const navigationItems: {
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
}[] = [
	{
		href: "/library/config",
		label: "Dashboard",
		icon: SquarePenIcon,
		description: "Overview, stats, and getting started",
		group: "General",
	},
	{
		href: "/library/config/configuration",
		label: "Configuration",
		icon: CogIcon,
		description: "App configuration and environment variables",
		group: "General",
	},
	{
		href: "/library/config/runtime-vars",
		label: "Runtime Variables",
		icon: KeyIcon,
		description: "User-specific runtime secrets and configurations",
		group: "General",
	},
	{
		href: "/library/config/flows",
		label: "Flows",
		icon: WorkflowIcon,
		description: "Business logic and workflow definitions",
		group: "Build",
	},
	{
		href: "/library/config/pages",
		label: "Events",
		icon: SparklesIcon,
		description: "Events, pages, and path-based navigation",
		group: "Build",
	},
	{
		href: "/library/config/templates",
		label: "Templates",
		icon: CopyIcon,
		description: "Reusable Flow templates",
		group: "Build",
	},
	{
		href: "/library/config/widgets",
		label: "Widgets",
		icon: LayoutGridIcon,
		description: "Reusable UI components and widgets",
		group: "Build",
	},
	{
		href: "/library/config/storage",
		label: "Storage",
		icon: FolderClosedIcon,
		description: "Data storage and file management",
		group: "Data",
	},
	{
		href: "/library/config/user-storage",
		label: "User Storage",
		icon: UserIcon,
		description: "Browse and search your private app files",
		group: "Data",
	},
	{
		href: "/library/config/explore",
		label: "Data Studio",
		icon: DatabaseIcon,
		description: "Model, explore, operate, and share project data",
		group: "Data",
	},
	{
		href: "/library/config/packages",
		label: "Packages",
		icon: PackageIcon,
		description: "Manage WASM packages for this app",
		group: "Data",
	},
	{
		href: "/library/config/team",
		label: "Team",
		icon: UsersRoundIcon,
		description: "Manage team members and permissions",
		visibilities: [
			IAppVisibility.Public,
			IAppVisibility.Prototype,
			IAppVisibility.PublicRequestAccess,
		],
		group: "Collaborate",
	},
	{
		href: "/library/config/suites",
		label: "Suites",
		icon: LayersIcon,
		description: "Bundle this app with related apps into one store listing",
		// A suite is presentation, not membership — private apps curate them too.
		visibilities: [
			IAppVisibility.Public,
			IAppVisibility.Prototype,
			IAppVisibility.PublicRequestAccess,
			IAppVisibility.Private,
		],
		group: "Collaborate",
	},
	{
		href: "/library/config/roles",
		label: "Roles",
		icon: CrownIcon,
		description: "Define user roles and access levels",
		visibilities: [
			IAppVisibility.Public,
			IAppVisibility.Prototype,
			IAppVisibility.PublicRequestAccess,
		],
		group: "Collaborate",
	},
	{
		href: "/library/config/sales",
		label: "Sales",
		icon: DollarSignIcon,
		description: "Track sales, manage pricing and discounts",
		visibilities: [IAppVisibility.Public, IAppVisibility.PublicRequestAccess],
		requiresPaid: true,
		group: "Insights",
	},
	{
		href: "/library/config/analytics",
		label: "Analytics",
		icon: ChartAreaIcon,
		description: "Performance metrics and insights",
		group: "Insights",
	},
	{
		href: "/library/config/endpoints",
		label: "Endpoints",
		icon: GlobeIcon,
		description: "API endpoints and integrations",
		group: "Insights",
	},
	{
		href: "/library/config/publication",
		label: "Publication",
		icon: SendIcon,
		description: "Track publication review status and auditor feedback",
		group: "Insights",
	},
];

function isRouteActive(itemHref: string, currentRoute: string): boolean {
	if (itemHref === "/library/config") {
		return currentRoute === "/library/config";
	}
	return currentRoute.startsWith(itemHref);
}

export default function Id({
	children,
}: Readonly<{ children: React.ReactNode }>) {
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

	// Use local visibility if available, otherwise fall back to app.data.visibility
	const effectiveVisibility = useMemo(
		() =>
			online?.visibility !== IAppVisibility.Offline
				? online?.visibility
				: (app.data?.visibility ?? IAppVisibility.Offline),
		[online?.visibility, app.data?.visibility],
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
					aria-label="Open menu"
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
					aria-label="Open menu"
				>
					<MenuIcon className="w-4 h-4" />
				</Button>,
				...(canUseApp
					? [
							<Link key={"use-app"} href={useAppHref} className="md:hidden">
								<Button variant="default" size="sm" aria-label="Use App">
									<SparklesIcon className="w-4 h-4" />
									Use App
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
		toast.info("Export not available", {
			description:
				"App export is only available in the desktop app. Your app is automatically synced to the cloud.",
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
				"Failed to execute board",
				<PlayCircleIcon className="w-4 h-4" />,
			);
		}
	}

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
											Home
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
												<h4 className="text-sm font-medium">Use App</h4>
											</Button>
										</Link>
									</div>
								)}

								{/* Mobile "Use App" quick action */}
								{useAppHref && (
									<Link href={useAppHref} className="md:hidden">
										<Button variant="default" size="sm" aria-label="Use App">
											<SparklesIcon className="w-4 h-4" />
											Use App
										</Button>
									</Link>
								)}

								<Button
									variant="outline"
									size="sm"
									className="md:hidden"
									onClick={() => setMobileNavOpen(true)}
									aria-label="Open menu"
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
							<DialogTitle>Navigation</DialogTitle>
							<DialogDescription>
								Quickly jump to settings sections
							</DialogDescription>
						</div>
						<ScrollArea className="max-h-[70vh]">
							<nav
								className="flex flex-col gap-1 p-3"
								key={id + (effectiveVisibility ?? "")}
							>
								{navigationItems
									.filter(
										(item) =>
											(!item.visibilities ||
												item.visibilities.includes(
													effectiveVisibility ?? IAppVisibility.Offline,
												)) &&
											(!item.requiresPaid ||
												(app.data?.price != null && app.data.price > 0)),
									)
									.map((item) => {
										const Icon = item.icon;
										if (item.disabled) {
											return (
												<div
													key={item.href}
													className="flex items-center gap-3 px-3 py-2 rounded-lg text-sm text-muted-foreground bg-muted/50 opacity-60"
													aria-disabled="true"
												>
													<Icon className="w-4 h-4 flex-shrink-0" />
													<span className="truncate">{item.label} (soon)</span>
												</div>
											);
										}
										return (
											<Link
												key={item.href}
												href={`${item.href}?id=${id}`}
												className="flex items-center gap-3 px-3 py-2 rounded-lg text-sm hover:bg-muted text-muted-foreground hover:text-foreground"
												onClick={() => setMobileNavOpen(false)}
											>
												<Icon className="w-4 h-4 flex-shrink-0" />
												<span className="truncate">{item.label}</span>
												{item.href === "/library/config/publication" &&
													hasActivePublicationRequest && (
														<span className="ml-auto w-2 h-2 rounded-full bg-blue-500 shrink-0" />
													)}
											</Link>
										);
									})}

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
										<span className="truncate">Export App</span>
									</Button>
								)}
							</nav>
						</ScrollArea>
					</DialogContent>
				</Dialog>

				{/* Global Export Dialog */}
				<Dialog open={exportOpen} onOpenChange={setExportOpen}>
					<DialogContent className="sm:max-w-[520px]">
						<DialogHeader>
							<DialogTitle>Export Application</DialogTitle>
							<DialogDescription>
								Choose how you want to export your app.
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
												? "Protect your export with a password."
												: "Quick export without encryption."}
										</p>
									</div>
								</div>
								<div className="flex items-center gap-2">
									<span className="text-xs text-muted-foreground">Encrypt</span>
									<Switch checked={encrypt} onCheckedChange={setEncrypt} />
								</div>
							</div>

							{encrypt && (
								<div className="space-y-3">
									<div className="grid gap-2">
										<Label htmlFor="export-password" className="text-xs">
											Password
										</Label>
										<div className="relative">
											<Input
												id="export-password"
												type={showPassword ? "text" : "password"}
												value={password}
												onChange={(e) => setPassword(e.target.value)}
												placeholder="Enter a strong password"
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
											Confirm password
										</Label>
										<Input
											id="export-password-confirm"
											type={showPassword ? "text" : "password"}
											value={confirmPassword}
											onChange={(e) => setConfirmPassword(e.target.value)}
											placeholder="Re-enter password"
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
											Passwords must match and be at least 8 characters.
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
								Cancel
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
											const filtered = navigationItems.filter(
												(item) =>
													(!item.visibilities ||
														item.visibilities.includes(
															effectiveVisibility ?? IAppVisibility.Offline,
														)) &&
													(!item.requiresPaid ||
														(app.data?.price != null && app.data.price > 0)),
											);
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
																		{item.label} (Coming soon!)
																	</p>
																	<p className="text-xs mt-1">
																		{item.description}
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
														<span className="truncate">Export App</span>
													</Button>
												</TooltipTrigger>
												<TooltipContent side="right" className="max-w-xs">
													<p className="font-bold">Export Application</p>
													<p className="text-xs mt-1">
														Export the application to a file for backup or
														sharing.
													</p>
												</TooltipContent>
											</Tooltip>
										)}
									</nav>

									<Separator className="my-4 mx-3" />

									<div className="px-3">
										<div className="flex items-center gap-2 mb-3">
											<ZapIcon className="w-4 h-4 text-primary" />
											<h4 className="text-sm font-medium">Quick Actions</h4>
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
													No quick actions available
												</p>
											)}
										</div>
									</div>
								</ScrollArea>
							</CardContent>
						</Card>
					)}

					<Card
						className={`relative h-full max-h-full flex-col grow overflow-hidden min-h-0 transition-all duration-300 bg-transparent hidden md:flex ${isMaximized ? "shadow-2xl" : ""} order-1 md:order-2`}
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
									<div className="px-6 pt-4 pr-16">
										<AppPublicationBanner
											requests={publicationRequests.data ?? []}
											onNavigate={() => {
												window.location.href = `/library/config/publication?id=${id}`;
											}}
										/>
									</div>
								)}
							{currentRoute?.includes("/storage") ||
							currentRoute?.includes("/explore") ? (
								<div className="h-full flex flex-col">
									<div className="flex-1 min-h-0 overflow-hidden p-6 pt-4 pr-16">
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
							) : (
								<div className="h-full overflow-y-auto">
									<div className="p-6 pt-4 pr-16">
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
							)}
						</CardContent>
					</Card>

					<div className="flex flex-col max-h-full md:hidden overflow-auto ">
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
			</main>
		</TooltipProvider>
	);
}
