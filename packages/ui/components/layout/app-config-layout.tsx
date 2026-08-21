"use client";

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
	UnlockIcon,
	ZapIcon,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import Link from "next/link";
import type { ReactNode } from "react";
import { Suspense, useCallback, useEffect, useMemo, useState } from "react";
import { configRouteFillsHeight } from "../../lib/config-route";
import {
	type IApp,
	IAppVisibility,
	type IEvent,
	type IMetadata,
} from "../../types";
import { VisibilityIcon } from "../ui/app-card";
import { Avatar, AvatarFallback, AvatarImage } from "../ui/avatar";
import { Badge } from "../ui/badge";
import {
	Breadcrumb,
	BreadcrumbItem,
	BreadcrumbLink,
	BreadcrumbList,
	BreadcrumbPage,
	BreadcrumbSeparator,
} from "../ui/breadcrumb";
import { Button } from "../ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "../ui/card";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import {
	HoverCard,
	HoverCardContent,
	HoverCardTrigger,
} from "../ui/hover-card";
import { Input } from "../ui/input";
import { Label } from "../ui/label";
import { ScrollArea } from "../ui/scroll-area";
import { Separator } from "../ui/separator";
import { Skeleton } from "../ui/skeleton";
import { Switch } from "../ui/switch";
import {
	Tooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../ui/tooltip";

export interface NavigationItem {
	href: string;
	label: string;
	icon: LucideIcon;
	description: string;
	visibilities?: IAppVisibility[];
	disabled?: boolean;
}

export interface AppConfigLayoutProps {
	appId: string;
	metadata?: IMetadata | null;
	app?: IApp | null;
	isMetadataLoading?: boolean;
	visibility: IAppVisibility;
	navigationItems: NavigationItem[];
	quickActions?: IEvent[];
	currentRoute: string;
	children: ReactNode;
	onExport?: (password?: string) => Promise<void>;
	onExecuteEvent?: (event: IEvent) => Promise<void>;
	onMobileNavOpen?: () => void;
	mobileNavContent?: ReactNode;
	headerRightContent?: ReactNode;
}

export function AppConfigLayout({
	appId,
	metadata,
	app,
	isMetadataLoading = false,
	visibility,
	navigationItems,
	quickActions = [],
	currentRoute,
	children,
	onExport,
	onExecuteEvent,
	mobileNavContent,
	headerRightContent,
}: AppConfigLayoutProps) {
	const [isMaximized, setIsMaximized] = useState(false);
	const [exportOpen, setExportOpen] = useState(false);
	const [encrypt, setEncrypt] = useState(false);
	const [password, setPassword] = useState("");
	const [confirmPassword, setConfirmPassword] = useState("");
	const [showPassword, setShowPassword] = useState(false);
	const [exporting, setExporting] = useState(false);
	const [mobileNavOpen, setMobileNavOpen] = useState(false);

	useEffect(() => {
		if (typeof window === "undefined") return;
		const apply = () => {
			const isDesktop = window.matchMedia("(min-width: 768px)").matches;
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
		if (!onExport) return;
		setExporting(true);
		try {
			await onExport(encrypt && password ? password : undefined);
			setExportOpen(false);
			setPassword("");
			setConfirmPassword("");
		} finally {
			setExporting(false);
		}
	}, [onExport, encrypt, password]);

	const filteredNavItems = useMemo(
		() =>
			navigationItems.filter(
				(item) => !item.visibilities || item.visibilities.includes(visibility),
			),
		[navigationItems, visibility],
	);

	const activeQuickActions = useMemo(
		() =>
			quickActions.filter((e) => e.event_type === "quick_action" && e.active),
		[quickActions],
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
											Home
										</BreadcrumbLink>
									</BreadcrumbItem>
									<BreadcrumbSeparator />
									<BreadcrumbItem>
										<BreadcrumbPage className="font-medium flex flex-row items-center gap-2">
											{isMetadataLoading ? (
												<Skeleton className="h-4 w-24" />
											) : (
												metadata?.name
											)}
											{app?.visibility && (
												<div className="bg-gray-600/40 dark:bg-background rounded-full">
													<VisibilityIcon visibility={app?.visibility} />
												</div>
											)}
										</BreadcrumbPage>
									</BreadcrumbItem>
								</BreadcrumbList>
							</Breadcrumb>
							<div className="flex items-center gap-2">
								{headerRightContent}
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
							{mobileNavContent || (
								<nav
									className="flex flex-col gap-1 p-3"
									key={appId + visibility}
								>
									{filteredNavItems.map((item) => {
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
												href={`${item.href}?id=${appId}`}
												className="flex items-center gap-3 px-3 py-2 rounded-lg text-sm hover:bg-muted text-muted-foreground hover:text-foreground"
												onClick={() => setMobileNavOpen(false)}
											>
												<Icon className="w-4 h-4 flex-shrink-0" />
												<span className="truncate">{item.label}</span>
											</Link>
										);
									})}

									{visibility === IAppVisibility.Offline && onExport && (
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
							)}
						</ScrollArea>
					</DialogContent>
				</Dialog>

				{/* Export Dialog */}
				{onExport && (
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
										<span className="text-xs text-muted-foreground">
											Encrypt
										</span>
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
				)}

				<div
					className={`grid w-full items-stretch gap-6 flex-1 overflow-hidden min-h-0 transition-all duration-300 ${isMaximized ? "grid-cols-1" : "md:grid-cols-[240px_1fr] lg:grid-cols-[260px_1fr]"}`}
				>
					{!isMaximized && (
						<Card className="h-full max-h-full overflow-hidden py-2 hidden md:flex md:flex-col md:flex-grow order-2 md:order-1">
							<CardHeader className="pb-2 pt-2 border-b relative h-fit">
								<div className="flex flex-col gap-3">
									<div className="flex items-center gap-2 w-full">
										<div className="relative">
											<Avatar className="w-9 h-9 border border-border/50 shadow-sm">
												<AvatarImage
													src={metadata?.icon ?? "/app-logo.webp"}
													alt={`${metadata?.name ?? appId} icon`}
												/>
												<AvatarFallback className="text-xs font-semibold">
													{(metadata?.name ?? appId ?? "Unknown")
														.substring(0, 2)
														.toUpperCase()}
												</AvatarFallback>
											</Avatar>
										</div>
										<div className="flex-1 min-w-0">
											<CardTitle className="text-sm truncate">
												{isMetadataLoading ? (
													<Skeleton className="h-4 w-24" />
												) : (
													metadata?.name
												)}
											</CardTitle>
										</div>
									</div>

									{metadata?.description && (
										<p className="text-xs text-muted-foreground leading-relaxed line-clamp-2">
											{metadata.description}
										</p>
									)}

									<div className="flex flex-col gap-2">
										{metadata?.tags && metadata.tags.length > 0 && (
											<div className="flex flex-wrap gap-1 mb-2">
												{metadata.tags.slice(0, 2).map((tag) => (
													<Badge
														key={tag}
														variant="secondary"
														className="text-xs px-2 py-0.5"
													>
														{tag}
													</Badge>
												))}
												{metadata.tags.length > 2 && (
													<Tooltip>
														<TooltipTrigger asChild>
															<Badge
																variant="outline"
																className="text-xs px-2 py-0.5"
															>
																+{metadata.tags.length - 2}
															</Badge>
														</TooltipTrigger>
														<TooltipContent side="right" className="max-w-xs">
															<div className="space-y-1">
																{metadata.tags.slice(2).map((tag) => (
																	<Badge
																		key={tag}
																		variant="secondary"
																		className="text-xs mr-1"
																	>
																		{tag}
																	</Badge>
																))}
															</div>
														</TooltipContent>
													</Tooltip>
												)}
											</div>
										)}
									</div>
								</div>
							</CardHeader>
							<CardContent className="flex-1 p-0 overflow-hidden">
								<ScrollArea className="h-full px-3 flex-1">
									<div className="pt-3">
										<CardTitle className="text-sm font-medium text-muted-foreground mb-3">
											Navigation
										</CardTitle>
									</div>
									<nav
										className="flex flex-col gap-1 pb-4"
										key={appId + visibility}
									>
										{filteredNavItems.map((item) => {
											const Icon = item.icon;
											if (item.disabled) {
												return (
													<Tooltip key={item.href} delayDuration={300}>
														<TooltipTrigger asChild>
															<div
																className="flex items-center gap-3 px-3 py-2 rounded-lg text-sm text-muted-foreground bg-muted/50 opacity-60 cursor-not-allowed"
																tabIndex={-1}
																aria-disabled="true"
															>
																<Icon className="w-4 h-4 flex-shrink-0" />
																<span className="truncate">{item.label}</span>
															</div>
														</TooltipTrigger>
														<TooltipContent side="right" className="max-w-xs">
															<p className="font-bold">
																{item.label} (Coming soon!)
															</p>
															<p className="text-xs mt-1">{item.description}</p>
														</TooltipContent>
													</Tooltip>
												);
											}
											return (
												<Tooltip key={item.href} delayDuration={300}>
													<TooltipTrigger asChild>
														<Link
															href={`${item.href}?id=${appId}`}
															className="flex items-center gap-3 px-3 py-2 rounded-lg text-sm transition-all hover:bg-muted text-muted-foreground hover:text-foreground"
														>
															<Icon className="w-4 h-4 flex-shrink-0" />
															<span className="truncate">{item.label}</span>
														</Link>
													</TooltipTrigger>
													<TooltipContent side="right" className="max-w-xs">
														<p className="font-bold">{item.label}</p>
														<p className="text-xs mt-1">{item.description}</p>
													</TooltipContent>
												</Tooltip>
											);
										})}
										{visibility === IAppVisibility.Offline && onExport && (
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

									{activeQuickActions.length > 0 && onExecuteEvent && (
										<>
											<Separator className="my-4 mx-3" />

											<div className="px-3">
												<div className="flex items-center gap-2 mb-3">
													<ZapIcon className="w-4 h-4 text-primary" />
													<h4 className="text-sm font-medium">Quick Actions</h4>
												</div>
												<div className="flex flex-col gap-2 pb-4">
													{activeQuickActions.map((event) => (
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
																	onClick={() => onExecuteEvent(event)}
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
													))}
												</div>
											</div>
										</>
									)}

									{activeQuickActions.length === 0 && (
										<>
											<Separator className="my-4 mx-3" />
											<div className="px-3">
												<div className="flex items-center gap-2 mb-3">
													<ZapIcon className="w-4 h-4 text-primary" />
													<h4 className="text-sm font-medium">Quick Actions</h4>
												</div>
												<p className="text-xs text-muted-foreground py-2">
													No quick actions available
												</p>
											</div>
										</>
									)}
								</ScrollArea>
							</CardContent>
						</Card>
					)}

					<Card
						className={`h-full max-h-full flex-col flex-grow overflow-hidden min-h-0 transition-all duration-300 bg-transparent hidden md:flex ${isMaximized ? "shadow-2xl" : ""} order-1 md:order-2`}
					>
						<CardHeader className="pb-0 pt-4 px-4 hidden md:block">
							<div className="flex items-center justify-between">
								<div className="flex-1" />
								<Tooltip>
									<TooltipTrigger asChild>
										<Button
											variant="ghost"
											size="sm"
											onClick={() => setIsMaximized(!isMaximized)}
											className="h-8 w-8 p-0"
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
						</CardHeader>
						<CardContent className="flex-1 p-0 overflow-hidden min-h-0">
							{configRouteFillsHeight(currentRoute) ? (
								<div className="h-full flex flex-col">
									<div className="flex-1 min-h-0 p-6 pb-0 pt-0">
										<Suspense
											fallback={
												<div className="space-y-4">
													<Skeleton className="h-8 w-full" />
													<Skeleton className="h-32 w-full" />
													<Skeleton className="h-24 w-full" />
												</div>
											}
										>
											{children}
										</Suspense>
									</div>
								</div>
							) : (
								<ScrollArea className="h-full">
									<div className="p-6 pb-0 pt-0">
										<Suspense
											fallback={
												<div className="space-y-4">
													<Skeleton className="h-8 w-full" />
													<Skeleton className="h-32 w-full" />
													<Skeleton className="h-24 w-full" />
												</div>
											}
										>
											{children}
										</Suspense>
									</div>
								</ScrollArea>
							)}
						</CardContent>
					</Card>

					<div className="flex flex-col max-h-full md:hidden overflow-auto">
						<Suspense
							fallback={
								<div className="space-y-4">
									<Skeleton className="h-8 w-full" />
									<Skeleton className="h-32 w-full" />
									<Skeleton className="h-24 w-full" />
								</div>
							}
						>
							{children}
						</Suspense>
					</div>
				</div>
			</main>
		</TooltipProvider>
	);
}
