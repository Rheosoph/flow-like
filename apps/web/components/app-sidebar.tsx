"use client";
import { createId } from "@paralleldrive/cuid2";
import * as Sentry from "@sentry/nextjs";
import {
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
} from "@tm9657/flow-like-ui";
import {
	BadgeCheck,
	BellIcon,
	BookOpenIcon,
	BrainCircuitIcon,
	BugIcon,
	Check,
	ChevronRight,
	ChevronsUpDown,
	CreditCard,
	Edit3Icon,
	HomeIcon,
	KeyIcon,
	KeyRoundIcon,
	LayoutDashboardIcon,
	LibraryIcon,
	LogInIcon,
	LogOut,
	type LucideIcon,
	Moon,
	Package2Icon,
	Plus,
	SettingsIcon,
	ShoppingBagIcon,
	SidebarCloseIcon,
	SidebarOpenIcon,
	Sparkles,
	Sun,
	UsersRoundIcon,
	Code2Icon,
	WorkflowIcon,
	ZapIcon,
} from "lucide-react";
import { useTheme } from "next-themes";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { useCallback, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { fetcher } from "../lib/api";
import { Shortcuts } from "./shortcuts";

const data = {
	navMain: [
		{
			title: "Home",
			url: "/",
			icon: HomeIcon,
			isActive: true,
			permission: false,
			items: [],
		},
		{
			title: "Explore Apps",
			url: "/store/explore/apps",
			icon: ShoppingBagIcon,
			isActive: false,
			permission: false,
			items: [],
		},
		{
			title: "Explore Models",
			url: "/settings/ai",
			icon: BrainCircuitIcon,
			isActive: false,
			permission: false,
			items: [],
		},
		{
			title: "Library",
			url: "/library",
			icon: LibraryIcon,
			isActive: false,
			permission: false,
			items: [],
		},
		{
			title: "User Actions",
			url: "/admin/user",
			icon: UsersRoundIcon,
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
			icon: LayoutDashboardIcon,
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
			icon: Package2Icon,
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
			icon: Sparkles,
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
			icon: KeyRoundIcon,
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
			url: "/settings/registry",
			icon: Code2Icon,
			isActive: false,
			items: [
				{
					title: "Owned",
					url: "/settings/registry/installed",
				},
				{
					title: "Explore",
					url: "/settings/registry/explore",
				},
			],
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
							<SidebarMenuButton>
								<BugIcon />
								<span>Report Bug</span>
							</SidebarMenuButton>
						</DialogTrigger>
						<DialogContent>
							<DialogHeader>
								<DialogTitle className="flex flex-row items-center gap-2">
									<BugIcon />
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
							<SidebarMenuButton>
								<Sun className="h-[1.2rem] w-[1.2rem] rotate-0 scale-100 transition-all dark:-rotate-90 dark:scale-0" />
								<Moon className="absolute h-[1.2rem] w-[1.2rem] rotate-90 scale-0 transition-all dark:rotate-0 dark:scale-100" />
								<span>{"Toggle Theme"}</span>
							</SidebarMenuButton>
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
						<SidebarMenuButton tooltip="Settings">
							<SettingsIcon className="size-4" />
							<span className="w-full flex flex-row items-center justify-between">
								Settings
							</span>
						</SidebarMenuButton>
					</a>
					<a
						href="https://docs.flow-like.com"
						target="_blank"
						rel="noopener noreferrer"
					>
						<SidebarMenuButton tooltip="Documentation">
							<BookOpenIcon className="size-4" />
							<span className="w-full flex flex-row items-center justify-between">
								Documentation{" "}
							</span>
						</SidebarMenuButton>
					</a>
					<SidebarMenuButton tooltip="Toggle Sidebar" onClick={toggleSidebar}>
						{open ? <SidebarCloseIcon /> : <SidebarOpenIcon />}
						<span className="w-full flex flex-row items-center justify-between">
							Toggle Sidebar{" "}
							<span className="ml-auto text-xs tracking-widest text-muted-foreground">
								⌘B
							</span>
						</span>
					</SidebarMenuButton>
				</div>
				<NavUser user={user} />
			</SidebarFooter>
			<SidebarRail />
		</Sidebar>
	);
}

function Profiles() {
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
					}),
				},
			);

			if (response.ok) {
				// Parse response to get the server-generated profile ID
				const createdProfile = await response.json();
				if (createdProfile?.id && typeof window !== "undefined") {
					localStorage.setItem("flow-like-profile-id", createdProfile.id);
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
						<SidebarMenuButton
							size="lg"
							className="data-[state=open]:bg-sidebar-accent data-[state=open]:text-sidebar-accent-foreground relative"
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
							<ChevronsUpDown className="ml-auto" />
						</SidebarMenuButton>
					</DropdownMenuTrigger>
					<DropdownMenuContent
						className="w-[--radix-dropdown-menu-trigger-width] min-w-56 rounded-lg"
						align="start"
						side={isMobile ? "bottom" : "right"}
						sideOffset={4}
					>
						<DropdownMenuLabel className="text-xs text-muted-foreground">
							Profile
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
										New profile
									</div>
								</DropdownMenuItem>
							</DialogTrigger>
							<DialogContent className="sm:max-w-md">
								<DialogHeader>
									<DialogTitle>Create New Profile</DialogTitle>
									<DialogDescription>
										Create a new profile to organize your apps and settings.
									</DialogDescription>
								</DialogHeader>
								<div className="grid gap-4 py-4">
									<div className="space-y-2">
										<Label htmlFor="new-profile-name">Name</Label>
										<Input
											id="new-profile-name"
											value={newProfileName}
											onChange={(e) => setNewProfileName(e.target.value)}
											placeholder="Profile name"
											autoFocus
										/>
									</div>
									<div className="space-y-2">
										<Label htmlFor="new-profile-description">
											Description (optional)
										</Label>
										<Textarea
											id="new-profile-description"
											value={newProfileDescription}
											onChange={(e) => setNewProfileDescription(e.target.value)}
											placeholder="Short description..."
											rows={3}
										/>
									</div>
								</div>
								<DialogFooter>
									<DialogClose asChild>
										<Button variant="ghost">Cancel</Button>
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
									Edit profile
								</div>
							</DropdownMenuItem>
						</a>
					</DropdownMenuContent>
				</DropdownMenu>
			</SidebarMenuItem>
		</SidebarMenu>
	);
}

interface INavItem {
	title: string;
	url: string;
	icon?: LucideIcon;
	isActive?: boolean;
	permission?: boolean;
	items?: {
		title: string;
		url: string;
		permission?: GlobalPermission;
	}[];
}

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
				<Link href={item.url}>
					{item.icon && <item.icon />}
					<span>{item.title}</span>
				</Link>
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
					<SidebarMenuButton
						variant={active ? "outline" : "default"}
						tooltip={item.title}
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
						{item.icon && <item.icon />}
						<span>{item.title}</span>
						<ChevronRight className="ml-auto transition-transform duration-200 group-data-[state=open]/collapsible:rotate-90" />
					</SidebarMenuButton>
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
	const router = useRouter();
	const pathname = usePathname();
	const { open } = useSidebar();
	const info = useInvoke(backend.userState.getInfo, backend.userState, []);

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
											<Link href={item.url}>
												{item.icon && <item.icon />}
												<span>{item.title}</span>
											</Link>
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
	);
	const info = useInvoke(backend.userState.getInfo, backend.userState, []);

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
		true,
		[],
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
						<SidebarMenuButton
							size="lg"
							className="data-[state=open]:bg-sidebar-accent data-[state=open]:text-sidebar-accent-foreground"
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
							<ChevronsUpDown className="ml-auto size-4" />
						</SidebarMenuButton>
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
													<Sparkles className="size-4" />
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

												window.open(
													urlRequest.url,
													"_blank",
													"noopener,noreferrer",
												);
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
									if (!auth) {
										toast.error("Authentication is not configured.");
										return;
									}
									try {
										console.log("[Login] Starting signinRedirect...");
										await auth.signinRedirect();
										console.log("[Login] signinRedirect completed");
									} catch (error) {
										console.error("[Login] signinRedirect failed:", error);
										toast.error(`Login failed: ${error}`);
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
							<SidebarMenuButton
								variant={pathname.startsWith("/flow") ? "outline" : "default"}
								tooltip={"Flows"}
								onClick={() => {
									const firstBoard = openBoards.data?.[0];
									if (firstBoard)
										router.push(
											`/flow?id=${firstBoard[1]}&app=${firstBoard[0]}`,
										);
								}}
							>
								<WorkflowIcon />
								<span>Open Flows</span>
								<ChevronRight className="ml-auto transition-transform duration-200 group-data-[state=open]/collapsible:rotate-90" />
							</SidebarMenuButton>
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
