"use client";
import { createId } from "@paralleldrive/cuid2";
import * as Sentry from "@sentry/nextjs";
import { invoke } from "@tauri-apps/api/core";
import { WebviewWindow } from "@tauri-apps/api/webviewWindow";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
	AnimatedBrainIcon,
	AnimatedBugIcon,
	AnimatedDashboardIcon,
	AnimatedDocsIcon,
	AnimatedFlowsIcon,
	AnimatedHomeIcon,
	AnimatedKeyIcon,
	AnimatedLibraryIcon,
	AnimatedPackageIcon,
	AnimatedSettingsIcon,
	AnimatedSparklesIcon,
	AnimatedExploreAppsIcon,
	AnimatedThemeIcon,
	AnimatedUsersIcon,
	Avatar,
	AvatarFallback,
	AvatarImage,
	Button,
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
	Dialog,
	DialogClose,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	DialogTrigger,
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuGroup,
	DropdownMenuItem,
	DropdownMenuLabel,
	DropdownMenuSeparator,
	DropdownMenuShortcut,
	DropdownMenuTrigger,
	FlowBackground,
	GlobalPermission,
	IBitTypes,
	Input,
	Label,
	MobileHeader,
	MobileHeaderProvider,
	Sidebar,
	SidebarContent,
	SidebarFooter,
	SidebarGroup,
	SidebarGroupLabel,
	SidebarHeader,
	SidebarInset,
	SidebarMenu,
	SidebarMenuButton,
	SidebarMenuItem,
	SidebarMenuSub,
	SidebarMenuSubButton,
	SidebarMenuSubItem,
	SidebarProvider,
	SidebarRail,
	SpotlightTrigger,
	Textarea,
	useBackend,
	useInvalidateInvoke,
	useInvoke,
	useSidebar,
	AnimatedSidebarIcon,
	AnimatedCodeIcon,
} from "@tm9657/flow-like-ui";
import type { ISettingsProfile } from "@tm9657/flow-like-ui/types";
import {
	BadgeCheck,
	BarChart3,
	BellIcon,
	BookOpenIcon,
	BugIcon,
	Check,
	ChevronRight,
	ChevronsUpDown,
	Code2Icon,
	CreditCard,
	Edit3Icon,
	HomeIcon,
	KeyIcon,

	LogInIcon,
	LogOut,
	type LucideIcon,
	Moon,
	Plus,
	SettingsIcon,
	SidebarCloseIcon,
	SidebarOpenIcon,
	Sun,
	Trash2Icon,
	WorkflowIcon,
	ZapIcon,
} from "lucide-react";
import { useTheme } from "next-themes";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useCallback, useEffect, useMemo, useRef, useState, type ComponentType } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { motion } from "framer-motion";
import { fetcher } from "../lib/api";
import { CreateProfileDialog } from "./add-profile";
import { Shortcuts } from "./shortcuts";
import { useTauriInvoke } from "./useInvoke";

const data = {
	navMain: [
		{
			title: "Home",
			url: "/",
			icon: AnimatedHomeIcon,
			isActive: true,
			permission: false,
			items: [],
		},
		{
			title: "Explore Apps",
			url: "/store/explore/apps",
			icon: AnimatedExploreAppsIcon,
			isActive: false,
			permission: false,
			items: [],
		},
		{
			title: "Explore Models",
			url: "/settings/ai",
			icon: AnimatedBrainIcon,
			isActive: false,
			permission: false,
			items: [],
		},
		{
			title: "Library",
			url: "/library",
			icon: AnimatedLibraryIcon,
			isActive: false,
			permission: false,
			items: [],
		},
		{
			title: "User Actions",
			url: "/admin/user",
			icon: AnimatedUsersIcon,
			permission: true,
			items: [
				{
					title: "Find",
					url: "/admin/user",
					permission: GlobalPermission.ReadProfile,
				},
				{
					title: "Manage",
					url: "/admin/user/edit",
					permission: GlobalPermission.WriteProfile,
				},
			],
		},
		{
			title: "Governance",
			url: "/admin/governance",
			icon: AnimatedDashboardIcon,
			permission: true,
			items: [
				{
					title: "Dashboard",
					url: "/admin/governance",
					permission: GlobalPermission.ReadPublishing,
				},
				{
					title: "Your Requests",
					url: "/admin/governance/requests",
					permission: GlobalPermission.WritePublishing,
				},
			],
		},
		{
			title: "Bits",
			url: "/admin/bits",
			icon: AnimatedPackageIcon,
			permission: true,
			items: [
				{
					title: "Add Bits",
					url: "/admin/bits/add",
					permission: GlobalPermission.WriteBits,
				},
				{
					title: "Add Profile",
					url: "/admin/profiles/add",
					permission: GlobalPermission.WriteBits,
				},
				{
					title: "Edit Bits",
					url: "/admin/bits/edit",
					permission: GlobalPermission.WriteBits,
				},
			],
		},
		{
			title: "Solutions",
			url: "/admin/solutions",
			icon: AnimatedSparklesIcon,
			permission: true,
			items: [
				{
					title: "Manage Requests",
					url: "/admin/solutions",
					permission: GlobalPermission.WriteSolutions,
				},
			],
		},
		{
			title: "Sinks",
			url: "/admin/sinks",
			icon: AnimatedKeyIcon,
			permission: true,
			items: [
				{
					title: "Service Tokens",
					url: "/admin/sinks",
					permission: GlobalPermission.Admin,
				},
			],
		},
	],
	navDev: [
		{
			title: "Developer",
			url: "/store/packages",
			icon: AnimatedPackageIcon,
			isActive: false,
		},
		{
			title: "Developer",
			url: "/developer/new",
			icon: AnimatedCodeIcon,
			isActive: false,
		},
	],
};

interface IUser {
	name: string;
	email: string;
	avatar: string;
}

export function AppSidebar({
	children,
}: Readonly<{ children: React.ReactNode }>) {
	// Guard localStorage usage for SSR and provide a sensible default.
	const defaultOpen =
		typeof window !== "undefined"
			? localStorage.getItem("sidebar_state") === "true"
			: true;

	return (
		<SidebarProvider defaultOpen={defaultOpen}>
			<InnerSidebar />
			<main
				className="w-full h-dvh flex flex-col overflow-hidden"
				style={{
					paddingTop: "var(--fl-safe-top, env(safe-area-inset-top, 0px))",
				}}
			>
				<MobileHeaderProvider>
					<MobileHeader />
					<SidebarInset className="relative flex flex-col flex-1 min-h-0 h-full overflow-hidden">
						<FlowBackground
							intensity="subtle"
							interactive
							className="flex flex-col flex-1 min-h-0 h-full"
						>
							{children}
						</FlowBackground>
					</SidebarInset>
				</MobileHeaderProvider>
			</main>
		</SidebarProvider>
	);
}

function IOSQuickMenuTrigger() {
	const { isMobile, openMobile, toggleSidebar } = useSidebar();
	const touchStartRef = useRef<{ x: number; y: number } | null>(null);

	const isIosTauri = useMemo(() => {
		if (typeof window === "undefined" || typeof navigator === "undefined") {
			return false;
		}

		const isTauri =
			"__TAURI__" in (window as any) ||
			"__TAURI_IPC__" in (window as any) ||
			"__TAURI_INTERNALS__" in (window as any);
		const isIOS =
			/iPad|iPhone|iPod/.test(navigator.userAgent) ||
			(navigator.platform === "MacIntel" && navigator.maxTouchPoints > 1);

		return isTauri && isIOS;
	}, []);

	useEffect(() => {
		if (!isIosTauri || !isMobile) return;

		const onTouchStart = (event: TouchEvent) => {
			const t = event.changedTouches[0];
			if (!t) return;
			touchStartRef.current = { x: t.clientX, y: t.clientY };
		};

		const onTouchEnd = (event: TouchEvent) => {
			if (openMobile) return;
			const start = touchStartRef.current;
			const t = event.changedTouches[0];
			if (!start || !t) return;

			const dx = t.clientX - start.x;
			const dy = Math.abs(t.clientY - start.y);

			// Left-edge swipe opens the menu if the header button is hard to tap.
			if (start.x <= 24 && dx > 40 && dy < 30) {
				toggleSidebar();
			}
		};

		window.addEventListener("touchstart", onTouchStart, { passive: true });
		window.addEventListener("touchend", onTouchEnd, { passive: true });

		return () => {
			window.removeEventListener("touchstart", onTouchStart);
			window.removeEventListener("touchend", onTouchEnd);
		};
	}, [isIosTauri, isMobile, openMobile, toggleSidebar]);

	if (!isIosTauri || !isMobile || openMobile) return null;

	return (
		<div
			className="md:hidden fixed left-3 z-[70]"
			style={{ top: "calc(var(--fl-safe-top, 0px) + 10px)" }}
		>
			<Button
				size="icon"
				variant="outline"
				className="h-10 w-10 rounded-lg shadow-lg bg-card/95 backdrop-blur supports-backdrop-filter:bg-background/70"
				onClick={toggleSidebar}
				onTouchStart={(event) => {
					event.stopPropagation();
				}}
				aria-label="Open menu"
			>
				<SidebarOpenIcon className="h-4 w-4" />
			</Button>
		</div>
	);
}

function InnerSidebar() {
	const router = useRouter();
	const [user] = useState<IUser | undefined>();
	const { open, toggleSidebar } = useSidebar();
	const { setTheme } = useTheme();
	const [feedback, setFeedback] = useState({
		name: "",
		email: "",
		message: "",
	});

	return (
		<Sidebar collapsible="icon" side="left">
			<SidebarHeader>
				<Profiles />
				<SpotlightTrigger />
			</SidebarHeader>
			<SidebarContent>
				<NavMain items={data.navMain} devItems={data.navDev} />
				<Shortcuts />
				<Flows />
			</SidebarContent>
			<SidebarFooter>
				<div className="flex flex-col gap-1">
					<Dialog>
						<DialogTrigger asChild>
							<MotionSidebarMenuButton initial="initial" whileHover="hover">
								<motion.div variants={iconVariants}>
									<AnimatedBugIcon />
								</motion.div>
								<span>Report Bug</span>
							</MotionSidebarMenuButton>
						</DialogTrigger>
						<DialogContent>
							<DialogHeader>
								<DialogTitle className="flex flex-row items-center gap-2">
									<AnimatedBugIcon />
									{"Bug Report"}
								</DialogTitle>
								<DialogDescription>
									{
										"Please describe the bug you encountered, Name and Email are optional."
									}
								</DialogDescription>
							</DialogHeader>
							<div className="grid gap-4 py-4">
								<div className="grid grid-cols-4 items-center gap-4">
									<Label htmlFor="name" className="text-right">
										{"Name (optional)"}
									</Label>
									<Input
										id="name"
										value={feedback.name}
										onChange={(e) =>
											setFeedback({ ...feedback, name: e.target.value })
										}
										className="col-span-3"
									/>
								</div>
								<div className="grid grid-cols-4 items-center gap-4">
									<Label htmlFor="username" className="text-right">
										{"Email (optional)"}
									</Label>
									<Input
										id="username"
										value={feedback.email}
										onChange={(e) =>
											setFeedback({ ...feedback, email: e.target.value })
										}
										className="col-span-3"
									/>
								</div>
								<div className="grid grid-cols-4 items-center gap-4">
									<Label htmlFor="message" className="text-right">
										{"Message"}
									</Label>
									<Textarea
										id="message"
										value={feedback.message}
										onChange={(e) =>
											setFeedback({ ...feedback, message: e.target.value })
										}
										className="col-span-3"
									/>
								</div>
							</div>

							<DialogFooter>
								<DialogClose asChild>
									<Button
										disabled={feedback.message === ""}
										onClick={() => {
											Sentry.captureFeedback(
												{
													name:
														feedback.name === "" ? undefined : feedback.name, // optional
													email:
														feedback.email === "" ? undefined : feedback.email, // optional
													message: feedback.message, // required
												},
												{
													includeReplay: true, // optional
												},
											);
											toast("Feedback sent successfully ❤️");
											setFeedback({ name: "", email: "", message: "" });
										}}
									>
										Send
									</Button>
								</DialogClose>
							</DialogFooter>
						</DialogContent>
					</Dialog>
					<DropdownMenu>
						<DropdownMenuTrigger asChild>
							<MotionSidebarMenuButton initial="initial" whileHover="hover">
								<motion.div variants={iconVariants}>
									<AnimatedThemeIcon />
								</motion.div>
								<span>{"Toggle Theme"}</span>
							</MotionSidebarMenuButton>
						</DropdownMenuTrigger>
						<DropdownMenuContent align="center" side="right">
							<DropdownMenuItem onClick={() => setTheme("light")}>
								{"Light"}
							</DropdownMenuItem>
							<DropdownMenuItem onClick={() => setTheme("dark")}>
								{"Dark"}
							</DropdownMenuItem>
							<DropdownMenuItem onClick={() => setTheme("system")}>
								{"System Default"}
							</DropdownMenuItem>
						</DropdownMenuContent>
					</DropdownMenu>

					<a href="/settings">
						<MotionSidebarMenuButton tooltip="Settings" initial="initial" whileHover="hover">
							<motion.div variants={iconVariants}>
								<AnimatedSettingsIcon className="size-4" />
							</motion.div>
							<span className="w-full flex flex-row items-center justify-between">
								Settings
							</span>
						</MotionSidebarMenuButton>
					</a>
					<a
						href="https://docs.flow-like.com"
						target="_blank"
						rel="noopener noreferrer"
					>
						<MotionSidebarMenuButton tooltip="Documentation" initial="initial" whileHover="hover">
							<motion.div variants={iconVariants}>
								<AnimatedDocsIcon className="size-4" />
							</motion.div>
							<span className="w-full flex flex-row items-center justify-between">
								Documentation{" "}
							</span>
						</MotionSidebarMenuButton>
					</a>
					<MotionSidebarMenuButton tooltip="Toggle Sidebar" onClick={toggleSidebar} initial="initial" whileHover="hover">
						<div>
							<AnimatedSidebarIcon className="size-4" isOpen={open} />
						</div>
						<span className="w-full flex flex-row items-center justify-between">
							Toggle Sidebar{" "}
							<span className="ml-auto text-xs tracking-widest text-muted-foreground">
								⌘B
							</span>
						</span>
					</MotionSidebarMenuButton>
				</div>
				<NavUser user={user} />
			</SidebarFooter>
			<SidebarRail />
		</Sidebar>
	);
}

function Profiles() {
	const [createProfile, setCreateProfile] = useState<boolean>(false);
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const { isMobile } = useSidebar();
	const profiles = useTauriInvoke<Record<string, ISettingsProfile>>(
		"get_profiles",
		{},
	);
	const currentProfile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const handleCreateProfile = useCallback(
		async (profile: ISettingsProfile) => {
			await invoke("upsert_profile", { profile });
			await profiles.refetch();
			await invalidate(backend.userState.getProfile, []);
			await currentProfile.refetch();
			if (profile.hub_profile.id) handleProfileChange(profile.hub_profile.id);
		},
		[setCreateProfile],
	);

	const handleProfileChange = useCallback(async (id: string) => {
		if (id !== "")
			await invoke("set_current_profile", {
				profileId: id,
			});
		await Promise.allSettled([
			invalidate(backend.userState.getProfile, []),
			invalidate(backend.userState.getSettingsProfile, []),
			invalidate(backend.appState.getApps, []),
			invalidate(backend.bitState.searchBits, [
				{
					bit_types: [
						IBitTypes.Llm,
						IBitTypes.Vlm,
						IBitTypes.Embedding,
						IBitTypes.ImageEmbedding,
					],
				},
			]),
			invalidate(backend.bitState.searchBits, [
				{
					bit_types: [IBitTypes.Template],
				},
			]),
		]);
	}, []);

	const [deleteTarget, setDeleteTarget] = useState<{
		id: string;
		name: string;
	} | null>(null);
	const [isDeleting, setIsDeleting] = useState(false);

	const handleDeleteProfile = useCallback(
		(profileId: string, profileName: string, e: React.MouseEvent) => {
			e.stopPropagation();
			e.preventDefault();
			const profileCount = Object.keys(profiles.data ?? {}).length;
			if (profileCount <= 1) {
				toast.error("Cannot delete your only profile");
				return;
			}
			setDeleteTarget({ id: profileId, name: profileName });
		},
		[profiles],
	);

	const confirmDeleteProfile = useCallback(async () => {
		if (!deleteTarget) return;
		setIsDeleting(true);
		try {
			await invoke("delete_profile", { profileId: deleteTarget.id });
			toast.success("Profile removed");
			await profiles.refetch();
			await invalidate(backend.userState.getProfile, []);
			await invalidate(backend.userState.getSettingsProfile, []);
		} catch (err) {
			toast.error(`${err}`);
		} finally {
			setIsDeleting(false);
			setDeleteTarget(null);
		}
	}, [deleteTarget, profiles, invalidate, backend.userState]);

	return (
		<SidebarMenu>
			<SidebarMenuItem>
				<DropdownMenu>
					<DropdownMenuTrigger asChild>
						<MotionSidebarMenuButton
							size="lg"
							className="data-[state=open]:bg-sidebar-accent data-[state=open]:text-sidebar-accent-foreground relative"
							initial="initial"
							whileHover="hover"
						>
							<div className="flex relative aspect-square size-8 items-center justify-center rounded-lg bg-sidebar-primary text-sidebar-primary-foreground">
								<Avatar className="h-8 w-8 rounded-lg">
									<AvatarImage
										className="rounded-lg size-8 w-8 h-8"
										src={
											currentProfile.data?.hub_profile.icon ??
											"/placeholder.webp"
										}
									/>
									<AvatarImage
										className="rounded-lg size-8 w-8 h-8"
										src="/app-logo.webp"
									/>
									<AvatarFallback>NA</AvatarFallback>
								</Avatar>
							</div>
							<div className="grid flex-1 text-left text-sm leading-tight pl-1">
								<span className="truncate font-semibold">
									{currentProfile.data?.hub_profile.name}
								</span>
								<span className="truncate text-xs">
									{currentProfile.data?.hub_profile.hub?.replaceAll(
										"https://",
										"",
									)}
								</span>
							</div>
							<motion.div variants={iconVariants}>
								<ChevronsUpDown className="ml-auto" />
							</motion.div>
						</MotionSidebarMenuButton>
					</DropdownMenuTrigger>
					<DropdownMenuContent
						className="w-[--radix-dropdown-menu-trigger-width] min-w-56 rounded-lg"
						align="start"
						side={isMobile ? "bottom" : "right"}
						sideOffset={4}
					>
						<DropdownMenuLabel className="text-xs text-muted-foreground">
							Profiles
						</DropdownMenuLabel>
						{profiles.data &&
							Object.values(profiles.data).map((profile, index) => {
								const isActive =
									profile.hub_profile.id ===
									currentProfile.data?.hub_profile.id;
								return (
									<DropdownMenuItem
										key={profile.hub_profile.id}
										onClick={async () => {
											if (profile.hub_profile.id)
												handleProfileChange(profile.hub_profile.id);
										}}
										className="group gap-2 p-2"
									>
										<div className="flex size-6 items-center justify-center rounded-sm border">
											<Avatar className="h-8 w-8 rounded-sm">
												<AvatarImage
													className="rounded-sm w-8 h-8"
													src={
														profile.hub_profile.icon ??
														"/thumbnail-placeholder.webp"
													}
												/>
												<AvatarImage
													className="rounded-sm w-8 h-8"
													src="/app-logo.webp"
												/>
												<AvatarFallback>NA</AvatarFallback>
											</Avatar>
										</div>
										<span className="flex-1 truncate">
											{profile.hub_profile.name}
										</span>
										{isActive && (
											<Check className="size-4 text-primary shrink-0" />
										)}
										{!isActive &&
											Object.keys(profiles.data ?? {}).length > 1 && (
												<button
													type="button"
													className="text-muted-foreground/40 hover:text-destructive hover:bg-destructive/10 transition-colors p-1 rounded shrink-0"
													onClick={(e) =>
														handleDeleteProfile(
															profile.hub_profile.id ?? "",
															profile.hub_profile.name ?? "Profile",
															e,
														)
													}
												>
													<Trash2Icon className="size-3.5" />
												</button>
											)}
										<DropdownMenuShortcut>⌘{index + 1}</DropdownMenuShortcut>
									</DropdownMenuItem>
								);
							})}
						<DropdownMenuSeparator />
						<DropdownMenuItem
							className="gap-2 p-2"
							onClick={() => setCreateProfile(true)}
						>
							<div className="flex size-6 items-center justify-center rounded-md border bg-background">
								<Plus className="size-4" />
							</div>
							<div className="font-medium text-muted-foreground">
								Add profile
							</div>
						</DropdownMenuItem>
						<a href="/settings/profiles">
							<DropdownMenuItem className="gap-2 p-2">
								<div className="flex size-6 items-center justify-center rounded-md border bg-background">
									<Edit3Icon className="size-4" />
								</div>
								<div className="font-medium text-muted-foreground">
									Edit profile
								</div>
							</DropdownMenuItem>
						</a>
					</DropdownMenuContent>
				</DropdownMenu>
			</SidebarMenuItem>
			<CreateProfileDialog
				open={createProfile}
				setOpen={setCreateProfile}
				onCreate={handleCreateProfile}
			/>
			<AlertDialog
				open={!!deleteTarget}
				onOpenChange={(open) => {
					if (!open) setDeleteTarget(null);
				}}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>Delete profile</AlertDialogTitle>
						<AlertDialogDescription>
							Are you sure you want to delete{" "}
							<span className="font-medium text-foreground">
								{deleteTarget?.name}
							</span>
							? This action cannot be undone.
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel disabled={isDeleting}>Cancel</AlertDialogCancel>
						<AlertDialogAction
							onClick={confirmDeleteProfile}
							disabled={isDeleting}
							className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
						>
							{isDeleting ? "Deleting…" : "Delete"}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</SidebarMenu>
	);
}

type NavIcon = LucideIcon | ComponentType<{ className?: string }>;

interface INavItem {
	title: string;
	url: string;
	icon?: NavIcon;
	isActive?: boolean;
	permission?: boolean;
	items?: {
		title: string;
		url: string;
		permission?: GlobalPermission;
	}[];
}

const MotionLink = motion.create(Link);
const MotionSidebarMenuButton = motion.create(SidebarMenuButton);

const iconVariants = {
	initial: { scale: 1, rotate: 0 },
	hover: {
		scale: 1.1,
		rotate: 5,
		transition: { type: "spring", stiffness: 400, damping: 10 },
	},
};

function isItemActive(item: INavItem, pathname: string): boolean {
	if (pathname === item.url) return true;
	if (
		item.items?.some(
			(sub) => pathname === sub.url || pathname.startsWith(`${sub.url}/`),
		)
	)
		return true;
	return pathname.startsWith(`${item.url}/`);
}

function NavFlatItem({
	item,
	pathname,
}: Readonly<{ item: INavItem; pathname: string }>) {
	const active = isItemActive(item, pathname);
	return (
		<SidebarMenuItem>
			<SidebarMenuButton
				asChild
				variant={active ? "outline" : "default"}
				tooltip={item.title}
			>
				<MotionLink href={item.url} initial="initial" whileHover="hover">
					{item.icon && (
						<motion.div variants={iconVariants}>
							<item.icon className="size-4" />
						</motion.div>
					)}
					<span>{item.title}</span>
				</MotionLink>
			</SidebarMenuButton>
		</SidebarMenuItem>
	);
}

function NavCollapsible({
	item,
	pathname,
	sidebarOpen,
	onNavigate,
}: Readonly<{
	item: INavItem;
	pathname: string;
	sidebarOpen: boolean;
	onNavigate: (url: string) => void;
}>) {
	const active = isItemActive(item, pathname);
	return (
		<Collapsible
			asChild
			defaultOpen={
				(localStorage.getItem(`sidebar:${item.title}`) ??
					(item.isActive ? "open" : "closed")) === "open"
			}
			onOpenChange={(isOpen) => {
				localStorage.setItem(
					`sidebar:${item.title}`,
					isOpen ? "open" : "closed",
				);
			}}
			className="group/collapsible"
		>
			<SidebarMenuItem>
				<CollapsibleTrigger asChild>
					<MotionSidebarMenuButton
						variant={active ? "outline" : "default"}
						tooltip={item.title}
						initial="initial"
						whileHover="hover"
						onClick={() => {
							if (!sidebarOpen) onNavigate(item.url);
						}}
						onMouseDown={async (e) => {
							if (e.button === 1) {
								e.preventDefault();
								try {
									const webview = new WebviewWindow(`sidebar-${createId()}`, {
										url: item.url,
										title: item.title,
										focus: true,
										resizable: true,
										maximized: false,
										width: 1200,
										height: 800,
									});
									webview.once("tauri://error", (error) => {
										console.error("Failed to open new window:", error);
									});
								} catch (error) {
									console.error("Failed to open new window:", error);
								}
							}
						}}
					>
						{item.icon && (
							<motion.div variants={iconVariants}>
								<item.icon className="size-4" />
							</motion.div>
						)}
						<span>{item.title}</span>
						<ChevronRight className="ml-auto transition-transform duration-200 group-data-[state=open]/collapsible:rotate-90" />
					</MotionSidebarMenuButton>
				</CollapsibleTrigger>
				<CollapsibleContent>
					<SidebarMenuSub>
						{item.items?.map((subItem) => (
							<SidebarMenuSubItem key={subItem.title}>
								<SidebarMenuSubButton asChild>
									<Link href={subItem.url}>
										<span
											className={
												pathname === subItem.url ||
												pathname.startsWith(`${subItem.url}/`)
													? "font-bold text-primary"
													: ""
											}
										>
											{subItem.title}
										</span>
									</Link>
								</SidebarMenuSubButton>
							</SidebarMenuSubItem>
						))}
					</SidebarMenuSub>
				</CollapsibleContent>
			</SidebarMenuItem>
		</Collapsible>
	);
}

function NavMain({
	items,
	devItems,
}: Readonly<{
	items: INavItem[];
	devItems: INavItem[];
}>) {
	const backend = useBackend();
	const auth = useAuth();
	const router = useRouter();
	const pathname = usePathname();
	const { open } = useSidebar();
	const info = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		Boolean(auth?.isAuthenticated),
		[auth?.user?.profile?.sub, auth?.isAuthenticated],
	);

	return (
		<>
			<SidebarGroup>
				<SidebarGroupLabel>Navigation</SidebarGroupLabel>
				<SidebarMenu>
					{items
						.filter((item) => !item.permission)
						.map((item) =>
							item.items && item.items.length > 0 ? (
								<NavCollapsible
									key={item.title}
									item={item}
									pathname={pathname}
									sidebarOpen={open}
									onNavigate={router.push}
								/>
							) : (
								<NavFlatItem key={item.title} item={item} pathname={pathname} />
							),
						)}
				</SidebarMenu>
			</SidebarGroup>
			<SidebarGroup>
				<SidebarGroupLabel>Development</SidebarGroupLabel>
				<SidebarMenu>
					{devItems.map((item) =>
						item.items && item.items.length > 0 ? (
							<NavCollapsible
								key={item.title}
								item={item}
								pathname={pathname}
								sidebarOpen={open}
								onNavigate={router.push}
							/>
						) : (
							<NavFlatItem key={item.title} item={item} pathname={pathname} />
						),
					)}
				</SidebarMenu>
			</SidebarGroup>
			{(info.data?.permission ?? 0) > 0 && (
				<SidebarGroup>
					<SidebarGroupLabel>Admin Area</SidebarGroupLabel>
					<SidebarMenu>
						{items
							.filter(
								(item) =>
									item.permission &&
									typeof item.items?.find((subitem) =>
										new GlobalPermission(
											info.data?.permission ?? 0,
										).hasPermission(
											subitem.permission ?? GlobalPermission.Admin,
										),
									) !== "undefined",
							)
							.map((item) =>
								item.items && item.items.length > 0 ? (
									<NavCollapsible
										key={item.title}
										item={{
											...item,
											items: item.items?.filter((sub) =>
												new GlobalPermission(
													info.data?.permission ?? 0,
												).hasPermission(
													sub.permission ?? GlobalPermission.Admin,
												),
											),
										}}
										pathname={pathname}
										sidebarOpen={open}
										onNavigate={router.push}
									/>
								) : (
									<SidebarMenuItem key={item.title}>
										<SidebarMenuButton
											asChild
											variant={pathname === item.url ? "outline" : "default"}
											tooltip={item.title}
										>
											<MotionLink href={item.url} initial="initial" whileHover="hover">
												{item.icon && (
													<motion.div variants={iconVariants}>
														<item.icon className="size-4" />
													</motion.div>
												)}
												<span>{item.title}</span>
											</MotionLink>
										</SidebarMenuButton>
									</SidebarMenuItem>
								),
							)}
					</SidebarMenu>
				</SidebarGroup>
			)}
		</>
	);
}

export function NavUser({
	user,
}: Readonly<{
	user?: IUser;
}>) {
	const { isMobile } = useSidebar();
	const auth = useAuth();
	const backend = useBackend();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		Boolean(auth?.isAuthenticated),
		[auth?.user?.profile?.sub, auth?.isAuthenticated],
	);
	const info = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		Boolean(auth?.isAuthenticated),
		[auth?.user?.profile?.sub, auth?.isAuthenticated],
	);

	const displayName: string = useMemo(() => {
		if (!info.data) return "Offline";

		return info.data?.name ?? info.data?.preferred_username ?? "Offline";
	}, [info.data]);

	const email: string = useMemo(() => {
		return info.data?.email ?? "Anonymous";
	}, [info.data]);

	const notifications = useInvoke(
		backend.userState.getNotifications,
		backend.userState,
		[],
		Boolean(auth?.isAuthenticated),
		[auth?.user?.profile?.sub, auth?.isAuthenticated],
		0, // staleTime: 0 to always refetch on mount
	);
	// Show total unread count (includes invites + local workflow notifications)
	const notificationCount =
		(notifications.data?.unread_count ?? 0) +
		(notifications.data?.invites_count ?? 0);

	return (
		<SidebarMenu>
			<SidebarMenuItem>
				<DropdownMenu>
					<DropdownMenuTrigger asChild>
						<MotionSidebarMenuButton
							size="lg"
							className="data-[state=open]:bg-sidebar-accent data-[state=open]:text-sidebar-accent-foreground"
							initial="initial"
							whileHover="hover"
						>
							<Avatar className="h-8 w-8 rounded-lg">
								<AvatarImage
									src={info.data?.avatar}
									alt={user?.name ?? "Offline"}
								/>
								<AvatarFallback className="rounded-lg">
									{displayName.slice(0, 2).toUpperCase()}
								</AvatarFallback>
							</Avatar>
							{notificationCount > 0 && (
								<div className="absolute -top-2 -right-2 bg-red-500 text-white text-xs rounded-full min-w-[16px] h-4 flex items-center justify-center px-1">
									{notificationCount > 5 ? "5+" : notificationCount}
								</div>
							)}
							<div className="grid flex-1 text-left text-sm leading-tight">
								<span className="truncate font-semibold">{displayName}</span>
								<span className="truncate text-xs">{email}</span>
							</div>
							<motion.div variants={iconVariants}>
								<ChevronsUpDown className="ml-auto size-4" />
							</motion.div>
						</MotionSidebarMenuButton>
					</DropdownMenuTrigger>
					<DropdownMenuContent
						className="w-[--radix-dropdown-menu-trigger-width] min-w-56 rounded-lg"
						side={isMobile ? "bottom" : "right"}
						align="end"
						sideOffset={4}
					>
						<DropdownMenuLabel className="p-0 font-normal">
							<div className="flex items-center gap-2 px-1 py-1.5 text-left text-sm">
								<Avatar className="h-8 w-8 rounded-lg">
									<AvatarImage src={info.data?.avatar} alt={"User Avatar"} />
									<AvatarFallback className="rounded-lg">
										{displayName.slice(0, 2).toUpperCase()}
									</AvatarFallback>
								</Avatar>
								<div className="grid flex-1 text-left text-sm leading-tight">
									<span className="truncate font-semibold">{displayName}</span>
									<span className="truncate text-xs">{email}</span>
								</div>
							</div>
						</DropdownMenuLabel>
						<DropdownMenuSeparator />
						{auth?.isAuthenticated && (
							<>
								{(!info.data?.tier ||
									info.data?.tier.toUpperCase() === "FREE") && (
									<>
										<DropdownMenuGroup>
											<a href="/subscription">
												<DropdownMenuItem className="gap-2">
													<AnimatedSparklesIcon />
													Upgrade to Pro
												</DropdownMenuItem>
											</a>
										</DropdownMenuGroup>
										<DropdownMenuSeparator />
									</>
								)}
								<DropdownMenuGroup>
									<a href="/account">
										<DropdownMenuItem className="gap-2">
											<BadgeCheck className="size-4" />
											Account
										</DropdownMenuItem>
									</a>
									{profile.data && (
										<DropdownMenuItem
											className="gap-2"
											onClick={async () => {
												const urlRequest = await fetcher<{ url: string }>(
													profile.data,
													"user/billing",
													{ method: "GET" },
													auth,
												);

												await openUrl(urlRequest.url);
											}}
										>
											<CreditCard className="size-4" />
											Billing
										</DropdownMenuItem>
									)}
									<a href="/notifications">
										<DropdownMenuItem className="gap-2 p-2">
											<div className="flex size-4relative">
												<BellIcon className="size-4" />
												{/* Add notification indicator */}
												{notificationCount > 0 && (
													<div className="absolute -top-0 -left-0 bg-red-500 text-white text-xs rounded-full min-w-[16px] h-4 flex items-center justify-center px-1">
														{notificationCount > 5 ? "5+" : notificationCount}
													</div>
												)}
											</div>
											Notifications
										</DropdownMenuItem>
									</a>
									<a href="/account/pat">
										<DropdownMenuItem className="gap-2 p-2">
											<KeyIcon className="size-4" />
											Token
										</DropdownMenuItem>
									</a>
									<a href="/settings/sinks">
										<DropdownMenuItem className="gap-2 p-2">
											<ZapIcon className="size-4" />
											Active Sinks
										</DropdownMenuItem>
									</a>
									<a href="/settings/statistics">
										<DropdownMenuItem className="gap-2 p-2">
											<BarChart3 className="size-4" />
											Board Statistics
										</DropdownMenuItem>
									</a>
								</DropdownMenuGroup>
								<DropdownMenuSeparator />
								<DropdownMenuItem
									className="gap-2"
									onClick={async () => {
										await auth?.signoutRedirect();
									}}
								>
									<LogOut className="size-4" />
									Log out
								</DropdownMenuItem>
							</>
						)}
						{!auth?.isAuthenticated && (
							<DropdownMenuItem
								className="gap-2"
								onClick={async () => {
									try {
										console.log("Signing in...");
										await auth?.signinRedirect();
										console.log("Sign-in initiated.");
									} catch (error) {
										console.error("Sign-in failed:", error);
									}
								}}
							>
								<LogInIcon className="size-4" />
								Log in
							</DropdownMenuItem>
						)}
					</DropdownMenuContent>
				</DropdownMenu>
			</SidebarMenuItem>
		</SidebarMenu>
	);
}

function Flows() {
	const backend = useBackend();
	const router = useRouter();
	const pathname = usePathname();
	const params = useSearchParams();
	const openBoards = useInvoke(
		backend.boardState.getOpenBoards,
		backend.boardState,
		[],
	);

	if ((openBoards.data?.length ?? 0) <= 0) return null;

	return (
		<SidebarGroup>
			<SidebarGroupLabel>Flows</SidebarGroupLabel>
			<SidebarMenu>
				<Collapsible
					asChild
					defaultOpen={localStorage.getItem("sidebar:flows") === "open"}
					onOpenChange={(open) => {
						localStorage.setItem("sidebar:flows", open ? "open" : "closed");
					}}
					className="group/collapsible"
				>
					<SidebarMenuItem>
						<CollapsibleTrigger asChild>
							<MotionSidebarMenuButton
								variant={pathname.startsWith("/flow") ? "outline" : "default"}
								tooltip={"Flows"}
								initial="initial"
								whileHover="hover"
								onClick={() => {
									const firstBoard = openBoards.data?.[0];
									if (firstBoard)
										router.push(
											`/flow?id=${firstBoard[1]}&app=${firstBoard[0]}`,
										);
								}}
							>
								<motion.div variants={iconVariants}>
									<AnimatedFlowsIcon />
								</motion.div>
								<span>Open Flows</span>
								<ChevronRight className="ml-auto transition-transform duration-200 group-data-[state=open]/collapsible:rotate-90" />
							</MotionSidebarMenuButton>
						</CollapsibleTrigger>
						<CollapsibleContent>
							<SidebarMenuSub>
								{openBoards.data?.map(([appId, boardId, boardName]) => (
									<SidebarMenuSubItem key={boardId}>
										<SidebarMenuSubButton asChild>
											<Link href={`/flow?id=${boardId}&app=${appId}`}>
												<span
													className={
														params.get("id") === boardId
															? "font-bold text-primary"
															: ""
													}
												>
													{boardName}
												</span>
											</Link>
										</SidebarMenuSubButton>
									</SidebarMenuSubItem>
								))}
							</SidebarMenuSub>
						</CollapsibleContent>
					</SidebarMenuItem>
				</Collapsible>
			</SidebarMenu>
		</SidebarGroup>
	);
}
