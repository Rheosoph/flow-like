"use client";
import {
	AnimatedBrainIcon,
	AnimatedDashboardIcon,
	AnimatedExploreAppsIcon,
	AnimatedFlowsIcon,
	AnimatedHomeIcon,
	AnimatedLibraryIcon,
	AnimatedSettingsIcon,
	AnimatedSidebarIcon,
	AnimatedSparklesIcon,
	AnimatedStudyHatIcon,
	AnimatedThemeIcon,
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
	LanguageSwitcher,
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
	Textarea,
	useBackend,
	useDeveloperMode,
	useInvalidateInvoke,
	useInvoke,
	useSidebar,
	userDisplayName,
	userInitials,
} from "@flow-like/flow-like-ui";
import { ownsWindowChrome } from "@flow-like/flow-like-ui/lib/chrome-route";
import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import { motion } from "framer-motion";
import {
	BadgeCheck,
	BellIcon,
	Check,
	ChevronRight,
	ChevronsUpDown,
	CreditCard,
	Edit3Icon,
	KeyIcon,
	LogInIcon,
	LogOut,
	type LucideIcon,
	Plus,
	ZapIcon,
} from "lucide-react";
import { useTheme } from "next-themes";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import {
	type ComponentType,
	useCallback,
	useEffect,
	useMemo,
	useState,
} from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { fetcher } from "../lib/api";
import { currentRelativeUrl } from "../lib/return-url";
import { Shortcuts } from "./shortcuts";

/** Labels are rebuilt per render so a language switch relabels the sidebar. */
function useNavData() {
	const { t } = useTranslation(["common", "settings"]);
	return useMemo(
		() => ({
			navMain: [
				{
					title: t("home", "Home"),
					url: "/",
					icon: AnimatedHomeIcon,
					isActive: true,
					permission: false,
					items: [],
				},
				{
					title: t("flowpilot", "FlowPilot"),
					url: "/chat",
					icon: AnimatedSparklesIcon,
					isActive: false,
					permission: false,
					items: [],
				},
				{
					title: t("explore", "Explore"),
					url: "/store/explore/apps",
					icon: AnimatedExploreAppsIcon,
					isActive: false,
					permission: false,
					items: [],
				},
				{
					title: t("exploreModels", "Explore Models"),
					url: "/settings/ai",
					icon: AnimatedBrainIcon,
					isActive: false,
					permission: false,
					devOnly: true,
					items: [],
				},
				{
					title: t("myApps", "My Apps"),
					url: "/library",
					icon: AnimatedLibraryIcon,
					isActive: false,
					permission: false,
					items: [],
				},
				{
					title: t("university", "University"),
					url: "/learn",
					icon: AnimatedStudyHatIcon,
					isActive: false,
					permission: false,
					items: [
						{
							title: t("overview", "Overview"),
							url: "/learn",
						},
						{
							title: t("settings:documentation", "Documentation"),
							url: "https://docs.flow-like.com",
							external: true,
						},
					],
				},
				{
					title: t("admin", "Admin"),
					url: "/admin",
					icon: AnimatedDashboardIcon,
					permission: true,
					items: [],
				},
			],
			navDev: [],
		}),
		[t],
	);
}

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
	const chromeless = ownsWindowChrome(usePathname());

	return (
		<SidebarProvider defaultOpen={defaultOpen} enableShortcut={!chromeless}>
			<GlobalChrome chromeless={chromeless} />
			<main
				className="w-full h-dvh flex flex-col overflow-hidden"
				style={{
					paddingTop: "var(--fl-safe-top, env(safe-area-inset-top, 0px))",
				}}
			>
				<MobileHeaderProvider>
					<MobileHeader />
					<SidebarInset
						className="relative flex flex-col flex-1 min-h-0 h-full overflow-hidden"
						style={{
							paddingBottom:
								"var(--fl-safe-bottom, env(safe-area-inset-bottom, 0px))",
						}}
					>
						<FlowBackground
							intensity="subtle"
							interactive
							active={!chromeless}
							className="flex flex-col flex-1 min-h-0"
						>
							{children}
						</FlowBackground>
					</SidebarInset>
				</MobileHeaderProvider>
			</main>
		</SidebarProvider>
	);
}

/**
 * The global sidebar, absent on routes that draw their own navigation.
 *
 * Unmounted rather than collapsed: `setOpen` writes `sidebar_state` to
 * localStorage unconditionally, so collapsing here would rewrite the user's
 * preference for every other route. Mobile keeps it — there it is a Radix Sheet
 * costing no layout space, and `MobileHeader`'s trigger is the only way to it.
 *
 * Must stay a child of `SidebarProvider`; `useSidebar` throws outside it.
 */
function GlobalChrome({ chromeless }: Readonly<{ chromeless: boolean }>) {
	const { isMobile, openMobile, setOpenMobile } = useSidebar();

	useEffect(() => {
		// A Sheet torn down mid-open strands `pointer-events: none` on the body.
		if (chromeless && !isMobile && openMobile) setOpenMobile(false);
	}, [chromeless, isMobile, openMobile, setOpenMobile]);

	if (chromeless && !isMobile) return null;
	return <InnerSidebar />;
}

function InnerSidebar() {
	const router = useRouter();
	const [user] = useState<IUser | undefined>();
	const { open, toggleSidebar } = useSidebar();
	const { setTheme } = useTheme();
	const { t } = useTranslation(["common", "settings"]);
	const data = useNavData();

	return (
		<Sidebar collapsible="icon" side="left">
			<SidebarHeader>
				<Profiles />
			</SidebarHeader>
			<SidebarContent>
				<NavMain items={data.navMain} devItems={data.navDev} />
				<Shortcuts />
				<Flows />
			</SidebarContent>
			<SidebarFooter>
				<div className="flex flex-col gap-1">
					<LanguageSwitcher />
					<DropdownMenu>
						<DropdownMenuTrigger asChild>
							<MotionSidebarMenuButton initial="initial" whileHover="hover">
								<motion.div variants={iconVariants}>
									<AnimatedThemeIcon />
								</motion.div>
								<span>{t("settings:theme.toggle")}</span>
							</MotionSidebarMenuButton>
						</DropdownMenuTrigger>
						<DropdownMenuContent align="center" side="right">
							<DropdownMenuItem onClick={() => setTheme("light")}>
								{t("settings:theme.light")}
							</DropdownMenuItem>
							<DropdownMenuItem onClick={() => setTheme("dark")}>
								{t("settings:theme.dark")}
							</DropdownMenuItem>
							<DropdownMenuItem onClick={() => setTheme("system")}>
								{t("settings:theme.system")}
							</DropdownMenuItem>
						</DropdownMenuContent>
					</DropdownMenu>

					<a href="/settings">
						<MotionSidebarMenuButton
							tooltip={t("settings", "Settings")}
							initial="initial"
							whileHover="hover"
						>
							<motion.div variants={iconVariants}>
								<AnimatedSettingsIcon className="size-4" />
							</motion.div>
							<span className="w-full flex flex-row items-center justify-between">
								{t("settings", "Settings")}
							</span>
						</MotionSidebarMenuButton>
					</a>
					<MotionSidebarMenuButton
						tooltip={t("toggleSidebar", "Toggle Sidebar")}
						onClick={toggleSidebar}
						initial="initial"
						whileHover="hover"
					>
						<div>
							<AnimatedSidebarIcon className="size-4" isOpen={open} />
						</div>
						<span className="w-full flex flex-row items-center justify-between">
							{t("toggleSidebar", "Toggle Sidebar")}{" "}
							<span className="ml-auto text-xs tracking-widest text-muted-foreground">
								{`⌘B`}
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
	const { t } = useTranslation("common");
	const backend = useBackend();
	const invalidate = useInvalidateInvoke();
	const { isMobile } = useSidebar();
	const auth = useAuth();
	const [createDialogOpen, setCreateDialogOpen] = useState(false);
	const [newProfileName, setNewProfileName] = useState("");
	const [newProfileDescription, setNewProfileDescription] = useState("");
	const [isCreating, setIsCreating] = useState(false);
	const currentProfile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);

	const allProfiles = useInvoke(
		backend.userState.getAllSettingsProfiles,
		backend.userState,
		[],
	);

	const profiles = allProfiles.data ?? [];

	const handleProfileChange = useCallback(
		async (id: string) => {
			// Save selected profile ID to localStorage
			if (typeof window !== "undefined") {
				localStorage.setItem("flow-like-profile-id", id);
			}
			await Promise.allSettled([
				invalidate(backend.userState.getProfile, []),
				invalidate(backend.userState.getSettingsProfile, []),
				invalidate(backend.appState.getApps, []),
				invalidate(backend.bitState.searchBits, [
					{
						bit_types: [
							IBitTypes.Llm,
							IBitTypes.Vlm,
							IBitTypes.Tts,
							IBitTypes.Stt,
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
		},
		[invalidate, backend],
	);

	const handleCreateProfile = useCallback(async () => {
		if (!newProfileName.trim() || !currentProfile.data) return;

		setIsCreating(true);
		try {
			const newProfileId = createId();
			const hubUrl =
				currentProfile.data.hub_profile.hub ||
				process.env.NEXT_PUBLIC_API_URL ||
				"https://api.flow-like.com";

			const response = await fetch(
				`${process.env.NEXT_PUBLIC_API_URL || "https://api.flow-like.com"}/api/v1/profile/${newProfileId}`,
				{
					method: "POST",
					headers: {
						"Content-Type": "application/json",
						...(auth?.user?.access_token && {
							Authorization: `Bearer ${auth.user.access_token}`,
						}),
					},
					body: JSON.stringify({
						name: newProfileName.trim(),
						description: newProfileDescription.trim() || null,
						hub: hubUrl,
						hubs: [hubUrl],
						home_default_id:
							currentProfile.data.hub_profile.home_default_id ?? null,
					}),
				},
			);

			if (response.ok) {
				const createdProfile = await response.json();
				const createdProfileId =
					createdProfile?.profile?.id ?? createdProfile?.id ?? newProfileId;
				if (createdProfileId && typeof window !== "undefined") {
					localStorage.setItem("flow-like-profile-id", createdProfileId);
				}

				// Refresh profile data
				await Promise.allSettled([
					invalidate(backend.userState.getProfile, []),
					invalidate(backend.userState.getSettingsProfile, []),
					invalidate(backend.userState.getAllSettingsProfiles, []),
				]);

				setCreateDialogOpen(false);
				setNewProfileName("");
				setNewProfileDescription("");
			}
		} finally {
			setIsCreating(false);
		}
	}, [
		newProfileName,
		newProfileDescription,
		currentProfile.data,
		auth,
		invalidate,
		backend,
	]);

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
							<div className="flex relative aspect-square size-8 items-center justify-center rounded-lg">
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
							{t("profile", "Profile")}
						</DropdownMenuLabel>
						{profiles
							.filter(
								(profile) => profile.hub_profile.id && profile.hub_profile.name,
							)
							.map((profile, index) => {
								const isCurrentProfile =
									profile.hub_profile.id ===
									currentProfile.data?.hub_profile.id;
								return (
									<DropdownMenuItem
										key={profile.hub_profile.id}
										onClick={async () => {
											if (profile.hub_profile.id)
												handleProfileChange(profile.hub_profile.id);
										}}
										className="gap-4 p-2"
									>
										<div className="flex size-6 items-center justify-center rounded-sm">
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
										{profile.hub_profile.name || "Unnamed Profile"}
										{isCurrentProfile && <Check className="ml-auto size-4" />}
										<DropdownMenuShortcut>⌘{index + 1}</DropdownMenuShortcut>
									</DropdownMenuItem>
								);
							})}
						<DropdownMenuSeparator />
						<Dialog open={createDialogOpen} onOpenChange={setCreateDialogOpen}>
							<DialogTrigger asChild>
								<DropdownMenuItem
									className="gap-2 p-2"
									onSelect={(e) => e.preventDefault()}
								>
									<div className="flex size-6 items-center justify-center rounded-md border bg-background">
										<Plus className="size-4" />
									</div>
									<div className="font-medium text-muted-foreground">
										{t("newProfile", "New profile")}
									</div>
								</DropdownMenuItem>
							</DialogTrigger>
							<DialogContent className="sm:max-w-md">
								<DialogHeader>
									<DialogTitle>
										{t("createNewProfile", "Create New Profile")}
									</DialogTitle>
									<DialogDescription>
										{`Create a new profile to organize your apps and settings.`}
									</DialogDescription>
								</DialogHeader>
								<div className="grid gap-4 py-4">
									<div className="space-y-2">
										<Label htmlFor="new-profile-name">Name</Label>
										<Input
											id="new-profile-name"
											value={newProfileName}
											onChange={(e) => setNewProfileName(e.target.value)}
											placeholder={t("profileName", "Profile name")}
											autoFocus
										/>
									</div>
									<div className="space-y-2">
										<Label htmlFor="new-profile-description">
											{t("descriptionOptional", "Description (optional)")}
										</Label>
										<Textarea
											id="new-profile-description"
											value={newProfileDescription}
											onChange={(e) => setNewProfileDescription(e.target.value)}
											placeholder={t(
												"shortDescription",
												"Short description...",
											)}
											rows={3}
										/>
									</div>
								</div>
								<DialogFooter>
									<DialogClose asChild>
										<Button variant="ghost">{t("cancel", "Cancel")}</Button>
									</DialogClose>
									<Button
										onClick={handleCreateProfile}
										disabled={!newProfileName.trim() || isCreating}
									>
										{isCreating ? "Creating..." : "Create"}
									</Button>
								</DialogFooter>
							</DialogContent>
						</Dialog>
						<a href="/settings/profiles">
							<DropdownMenuItem className="gap-2 p-2">
								<div className="flex size-6 items-center justify-center rounded-md border bg-background">
									<Edit3Icon className="size-4" />
								</div>
								<div className="font-medium text-muted-foreground">
									{t("editProfile", "Edit profile")}
								</div>
							</DropdownMenuItem>
						</a>
					</DropdownMenuContent>
				</DropdownMenu>
			</SidebarMenuItem>
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
	devOnly?: boolean;
	items?: {
		title: string;
		url: string;
		external?: boolean;
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
	return item.url !== "/" && pathname.startsWith(`${item.url}/`);
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
						onMouseDown={(e) => {
							if (e.button === 1) {
								e.preventDefault();
								window.open(item.url, "_blank", "noopener,noreferrer");
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
									{subItem.external ? (
										<a
											href={subItem.url}
											target="_blank"
											rel="noopener noreferrer"
										>
											<span>{subItem.title}</span>
										</a>
									) : (
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
									)}
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
	const { t } = useTranslation("common");
	const backend = useBackend();
	const router = useRouter();
	const pathname = usePathname();
	const { open } = useSidebar();
	const { developerMode } = useDeveloperMode();
	const info = useInvoke(backend.userState.getInfo, backend.userState, []);

	return (
		<>
			<SidebarGroup>
				<SidebarGroupLabel>{t("navigation", "Navigation")}</SidebarGroupLabel>
				<SidebarMenu>
					{items
						.filter((item) => !item.permission)
						.filter((item) => !item.devOnly || developerMode)
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
			{devItems.length > 0 && (
				<SidebarGroup>
					<SidebarGroupLabel>
						{t("development", "Development")}
					</SidebarGroupLabel>
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
			)}
			{(info.data?.permission ?? 0) > 0 && (
				<SidebarGroup>
					<SidebarGroupLabel>{t("adminArea", "Admin Area")}</SidebarGroupLabel>
					<SidebarMenu>
						{items
							.filter(
								(item) =>
									item.permission &&
									(!item.items?.length ||
										typeof item.items.find((subitem) =>
											new GlobalPermission(
												info.data?.permission ?? 0,
											).hasPermission(
												subitem.permission ?? GlobalPermission.Admin,
											),
										) !== "undefined"),
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
											<MotionLink
												href={item.url}
												initial="initial"
												whileHover="hover"
											>
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
	const { t } = useTranslation("common");
	const { isMobile } = useSidebar();
	const auth = useAuth();
	const backend = useBackend();
	const { developerMode } = useDeveloperMode();
	const authQueryDeps = [auth?.user?.profile?.sub, auth?.isAuthenticated];
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		Boolean(auth?.isAuthenticated),
		authQueryDeps,
	);
	const info = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		Boolean(auth?.isAuthenticated),
		authQueryDeps,
	);

	const displayName: string = useMemo(
		() => userDisplayName(info.data, "Offline"),
		[info.data],
	);

	const initials: string = useMemo(
		() => userInitials(displayName, "?"),
		[displayName],
	);

	const email: string = useMemo(() => {
		return info.data?.email ?? "Anonymous";
	}, [info.data]);

	const notifications = useInvoke(
		backend.userState.getNotifications,
		backend.userState,
		[],
		Boolean(auth?.isAuthenticated),
		authQueryDeps,
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
								<AvatarImage src={info.data?.avatar} alt={displayName} />
								<AvatarFallback className="rounded-lg">
									{initials}
								</AvatarFallback>
							</Avatar>
							{notificationCount > 0 && (
								<div className="absolute -top-2 -right-2 bg-primary text-primary-foreground text-xs rounded-full min-w-4 h-4 flex items-center justify-center px-1">
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
									<AvatarImage src={info.data?.avatar} alt={displayName} />
									<AvatarFallback className="rounded-lg">
										{initials}
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
													{t("upgradeToPro", "Upgrade to Pro")}
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
											{t("account", "Account")}
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

												window.open(
													urlRequest.url,
													"_blank",
													"noopener,noreferrer",
												);
											}}
										>
											<CreditCard className="size-4" />
											{t("billing", "Billing")}
										</DropdownMenuItem>
									)}
									<a href="/notifications">
										<DropdownMenuItem className="gap-2 p-2">
											<div className="flex size-4relative">
												<BellIcon className="size-4" />
												{/* Add notification indicator */}
												{notificationCount > 0 && (
													<div className="absolute top-0 left-0 bg-primary text-primary-foreground text-xs rounded-full min-w-4 h-4 flex items-center justify-center px-1">
														{notificationCount > 5 ? "5+" : notificationCount}
													</div>
												)}
											</div>
											{t("notifications", "Notifications")}
										</DropdownMenuItem>
									</a>
									{developerMode && (
										<>
											<a href="/account/pat">
												<DropdownMenuItem className="gap-2 p-2">
													<KeyIcon className="size-4" />
													{t("token", "Token")}
												</DropdownMenuItem>
											</a>
											<a href="/settings/sinks">
												<DropdownMenuItem className="gap-2 p-2">
													<ZapIcon className="size-4" />
													{t("activeSinks", "Active Sinks")}
												</DropdownMenuItem>
											</a>
										</>
									)}
								</DropdownMenuGroup>
								<DropdownMenuSeparator />
								<DropdownMenuItem
									className="gap-2"
									onClick={async () => {
										await auth?.signoutRedirect();
									}}
								>
									<LogOut className="size-4" />
									{t("logOut", "Log out")}
								</DropdownMenuItem>
							</>
						)}
						{!auth?.isAuthenticated && (
							<DropdownMenuItem
								className="gap-2"
								onClick={async () => {
									if (!auth) {
										toast.error("Authentication is not configured.");
										return;
									}
									try {
										console.log("[Login] Starting signinRedirect...");
										await auth.signinRedirect({
											url_state: currentRelativeUrl(),
										});
										console.log("[Login] signinRedirect completed");
									} catch (error) {
										console.error("[Login] signinRedirect failed:", error);
										toast.error(`Login failed: ${error}`);
									}
								}}
							>
								<LogInIcon className="size-4" />
								{t("logIn", "Log in")}
							</DropdownMenuItem>
						)}
					</DropdownMenuContent>
				</DropdownMenu>
			</SidebarMenuItem>
		</SidebarMenu>
	);
}

function Flows() {
	const { t } = useTranslation("common");
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
			<SidebarGroupLabel>{t("flows", "Flows")}</SidebarGroupLabel>
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
								tooltip={t("flows", "Flows")}
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
								<span>{t("openFlows", "Open Flows")}</span>
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
