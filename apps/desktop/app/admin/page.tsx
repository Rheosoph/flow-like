"use client";

import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	GlobalPermission,
	Skeleton,
	useBackend,
	useInvoke,
	useQuery,
	type IProfile,
} from "@tm9657/flow-like-ui";
import type { ISolutionListResponse } from "@tm9657/flow-like-ui";
import {
	BookOpen,
	Box,
	CheckCircle,
	Clock,
	Cpu,
	Download,
	Key,
	Lightbulb,
	Lock,
	Package,
	Plus,
	Shield,
	UserCog,
	Users,
	type LucideIcon,
} from "lucide-react";
import Link from "next/link";
import { useMemo } from "react";
import { useAuth } from "react-oidc-context";

function StatCard({
	title,
	value,
	description,
	icon,
	loading,
	href,
}: {
	title: string;
	value: number | string;
	description: string;
	icon: React.ReactNode;
	loading: boolean;
	href?: string;
}) {
	const inner = (
		<Card className="transition-colors hover:border-primary/40">
			<CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
				<CardTitle className="text-sm font-medium">{title}</CardTitle>
				{icon}
			</CardHeader>
			<CardContent>
				{loading ? (
					<Skeleton className="h-8 w-16" />
				) : (
					<div className="text-2xl font-bold">{value}</div>
				)}
				<p className="mt-1 text-xs text-muted-foreground">{description}</p>
			</CardContent>
		</Card>
	);

	return href ? <Link href={href}>{inner}</Link> : inner;
}

interface AdminSection {
	title: string;
	description: string;
	icon: LucideIcon;
	href: string;
	permission: GlobalPermission;
	actionLabel: string;
	color: string;
	links?: { label: string; href: string }[];
}

const ADMIN_SECTIONS: AdminSection[] = [
	{
		title: "Bits & Models",
		description: "Add hosted LLMs, manage existing bits, and edit model metadata.",
		icon: Cpu,
		href: "/admin/bits/add",
		permission: GlobalPermission.WriteBits,
		actionLabel: "Add Hosted LLM",
		color: "text-yellow-500",
		links: [
			{ label: "Add Bit", href: "/admin/bits/add" },
			{ label: "Edit Bits", href: "/admin/bits/edit" },
		],
	},
	{
		title: "Packages",
		description: "Review pending WASM packages and manage the package registry.",
		icon: Package,
		href: "/admin/packages",
		permission: GlobalPermission.ManagePackages,
		actionLabel: "Review Queue",
		color: "text-green-500",
	},
	{
		title: "Governance",
		description: "Review publication requests and manage app submissions.",
		icon: BookOpen,
		href: "/admin/governance",
		permission: GlobalPermission.ReadPublishing,
		actionLabel: "Publication Requests",
		color: "text-orange-500",
		links: [
			{ label: "Overview", href: "/admin/governance" },
			{ label: "Review Queue", href: "/admin/governance/requests" },
		],
	},
	{
		title: "Profile Templates",
		description: "Create and manage reusable profile templates for users.",
		icon: Users,
		href: "/admin/user",
		permission: GlobalPermission.ReadProfile,
		actionLabel: "Manage Templates",
		color: "text-purple-500",
		links: [
			{ label: "Browse", href: "/admin/user" },
			{ label: "Manage", href: "/admin/user/edit" },
			{ label: "Create", href: "/admin/profiles/add" },
		],
	},
	{
		title: "User Management",
		description: "Search users, manage tiers, permissions, and account status.",
		icon: UserCog,
		href: "/admin/users",
		permission: GlobalPermission.Admin,
		actionLabel: "Manage Users",
		color: "text-blue-500",
	},
	{
		title: "Solutions",
		description: "Review and manage solution requests from users.",
		icon: Lightbulb,
		href: "/admin/solutions",
		permission: GlobalPermission.ReadSolutions,
		actionLabel: "Manage Requests",
		color: "text-cyan-500",
	},
	{
		title: "Service Tokens",
		description: "Manage sink service tokens and API access credentials.",
		icon: Key,
		href: "/admin/sinks",
		permission: GlobalPermission.Admin,
		actionLabel: "Manage Tokens",
		color: "text-rose-500",
	},
];

function SectionCard({
	section,
	hasAccess,
}: {
	section: AdminSection;
	hasAccess: boolean;
}) {
	const Icon = section.icon;

	if (!hasAccess) {
		return (
			<Card className="opacity-50 pointer-events-none select-none">
				<CardHeader>
					<CardTitle className="flex items-center gap-2 text-base">
						<Lock className="h-4 w-4 text-muted-foreground" />
						{section.title}
					</CardTitle>
					<CardDescription>{section.description}</CardDescription>
				</CardHeader>
				<CardContent>
					<Badge variant="outline" className="text-xs text-muted-foreground">
						Insufficient permissions
					</Badge>
				</CardContent>
			</Card>
		);
	}

	return (
		<Card className="transition-colors hover:border-primary/40">
			<CardHeader>
				<CardTitle className="flex items-center gap-2 text-base">
					<Icon className={`h-4 w-4 ${section.color}`} />
					{section.title}
				</CardTitle>
				<CardDescription>{section.description}</CardDescription>
			</CardHeader>
			<CardContent className="space-y-3">
				<Button asChild size="sm" variant="outline" className="w-full">
					<Link href={section.href}>
						<Plus className="mr-2 h-3 w-3" />
						{section.actionLabel}
					</Link>
				</Button>
				{section.links && section.links.length > 0 && (
					<div className="flex flex-wrap gap-1.5">
						{section.links.map((link) => (
							<Link key={link.href} href={link.href}>
								<Badge
									variant="secondary"
									className="cursor-pointer hover:bg-accent transition-colors text-xs"
								>
									{link.label}
								</Badge>
							</Link>
						))}
					</div>
				)}
			</CardContent>
		</Card>
	);
}

export default function AdminDashboardPage() {
	const backend = useBackend();
	const auth = useAuth();
	const profile = useInvoke(backend.userState.getProfile, backend.userState, []);
	const info = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		Boolean(auth?.isAuthenticated),
		[auth?.user?.profile?.sub, auth?.isAuthenticated],
	);

	const perms = useMemo(
		() => new GlobalPermission(info.data?.permission ?? 0),
		[info.data?.permission],
	);

	const packageStats = useQuery<{
		totalPackages: number;
		totalVersions: number;
		totalDownloads: number;
		pendingReview: number;
		activePackages: number;
		rejectedPackages: number;
	}>({
		queryKey: ["admin", "packages", "stats"],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get(profile.data, "admin/packages/stats");
		},
		enabled: !!profile.data,
	});

	const profiles = useQuery<IProfile[]>({
		queryKey: ["info", "profiles"],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<IProfile[]>(profile.data, "info/profiles");
		},
		enabled: !!profile.data,
	});

	const openSolutions = useQuery<ISolutionListResponse>({
		queryKey: ["admin", "solutions", "open-count"],
		queryFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.get<ISolutionListResponse>(
				profile.data,
				"admin/solutions?page=1&limit=1&status=PENDING_REVIEW",
			);
		},
		enabled: !!profile.data,
	});

	const statsLoading = packageStats.isLoading;

	return (
		<main className="flex h-full min-h-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="flex-1 overflow-y-auto p-6">
				<div className="mx-auto max-w-6xl space-y-6">
					<div>
						<h1 className="text-3xl font-bold">Admin Dashboard</h1>
						<p className="text-muted-foreground">
							Central hub for registry management.
						</p>
					</div>

					{/* Pending review alert */}
					{(packageStats.data?.pendingReview ?? 0) > 0 && (
						<Card className="border-yellow-500/50 bg-yellow-500/5">
							<CardHeader className="pb-3">
								<CardTitle className="flex items-center gap-2 text-base text-yellow-700 dark:text-yellow-400">
									<Clock className="h-4 w-4" />
									{packageStats.data?.pendingReview} package
									{(packageStats.data?.pendingReview ?? 0) > 1 ? "s" : ""}{" "}
									pending review
								</CardTitle>
								<CardDescription>
									Packages are waiting for approval before they can be published.
								</CardDescription>
							</CardHeader>
							<CardContent>
								<Button asChild variant="outline" size="sm">
									<Link href="/admin/packages">Review Now</Link>
								</Button>
							</CardContent>
						</Card>
					)}

					{/* Stats overview */}
					<div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-5">
						<StatCard
							title="Pending Review"
							value={packageStats.data?.pendingReview ?? 0}
							description="Packages awaiting review"
							icon={<Clock className="h-4 w-4 text-yellow-500" />}
							loading={statsLoading}
							href="/admin/packages"
						/>
						<StatCard
							title="Open Solutions"
							value={openSolutions.isLoading ? "\u2014" : (openSolutions.data?.total ?? 0)}
							description="Solution requests pending"
							icon={<Lightbulb className="h-4 w-4 text-cyan-500" />}
							loading={openSolutions.isLoading}
							href="/admin/solutions"
						/>
						<StatCard
							title="Active Packages"
							value={packageStats.data?.activePackages ?? 0}
							description="Published and available"
							icon={<CheckCircle className="h-4 w-4 text-green-500" />}
							loading={statsLoading}
							href="/admin/packages"
						/>
						<StatCard
							title="Total Downloads"
							value={(packageStats.data?.totalDownloads ?? 0).toLocaleString()}
							description="Across all packages"
							icon={<Download className="h-4 w-4 text-blue-500" />}
							loading={statsLoading}
						/>
						<StatCard
							title="Profile Templates"
							value={profiles.isLoading ? "\u2014" : (profiles.data?.length ?? 0)}
							description="Reusable user profiles"
							icon={<Users className="h-4 w-4 text-purple-500" />}
							loading={profiles.isLoading}
							href="/admin/user/edit"
						/>
					</div>

					{/* Secondary stats */}
					<div className="grid gap-4 sm:grid-cols-3">
						<StatCard
							title="Total Packages"
							value={packageStats.data?.totalPackages ?? 0}
							description="All-time registered packages"
							icon={<Package className="h-4 w-4 text-muted-foreground" />}
							loading={statsLoading}
						/>
						<StatCard
							title="Total Versions"
							value={packageStats.data?.totalVersions ?? 0}
							description="Published package versions"
							icon={<Box className="h-4 w-4 text-muted-foreground" />}
							loading={statsLoading}
						/>
						<StatCard
							title="Rejected Packages"
							value={packageStats.data?.rejectedPackages ?? 0}
							description="Packages that failed review"
							icon={<Shield className="h-4 w-4 text-destructive" />}
							loading={statsLoading}
						/>
					</div>

					{/* Admin sections */}
					<div>
						<h2 className="mb-3 text-lg font-semibold">Manage</h2>
						<div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
							{ADMIN_SECTIONS.map((section) => (
								<SectionCard
									key={section.title}
									section={section}
									hasAccess={perms.hasPermission(section.permission)}
								/>
							))}
						</div>
					</div>
				</div>
			</div>
		</main>
	);
}
