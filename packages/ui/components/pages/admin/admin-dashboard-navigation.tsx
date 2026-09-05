"use client";
import { useTranslation } from "@flow-like/locales";
import {
	Activity,
	ArrowUpRight,
	BellRing,
	BookOpen,
	Bug,
	Cpu,
	GitBranch,
	GraduationCap,
	HardDrive,
	Key,
	Lightbulb,
	type LucideIcon,
	MessageSquareHeart,
	Package,
	Search,
	ShieldAlert,
	SlidersHorizontal,
	UserCog,
	Users,
	Waypoints,
	X,
} from "lucide-react";
import Link from "next/link";
import { useState } from "react";
import { GlobalPermission } from "../../../lib/permission/global-permission";
import { Button } from "../../ui/button";
import { Input } from "../../ui/input";

export interface AdminSection {
	title: string;
	description: string;
	icon: LucideIcon;
	href: string;
	permission: GlobalPermission;
	alternatePermissions?: GlobalPermission[];
	links?: { label: string; href: string }[];
	/** When set, the section is only shown if the named hub feature is on. */
	feature?: string;
}

export const ADMIN_SECTIONS: AdminSection[] = [
	{
		title: "Bits & Models",
		description:
			"Add hosted LLMs, manage existing bits, and edit model metadata.",
		icon: Cpu,
		href: "/admin/bits/edit",
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
		description:
			"Review pending WASM packages and manage the package registry.",
		icon: Package,
		href: "/admin/packages",
		permission: GlobalPermission.ManagePackages,
		actionLabel: "Review Queue",
		color: "text-green-500",
	},
	{
		title: "Governance",
		description:
			"Review app and suite publication requests and manage submissions.",
		icon: BookOpen,
		href: "/admin/governance",
		permission: GlobalPermission.ReadPublishing,
		actionLabel: "Publication Requests",
		color: "text-orange-500",
		links: [
			{ label: "Overview", href: "/admin/governance" },
			{ label: "Review Queue", href: "/admin/governance" },
			{ label: "Suites", href: "/admin/governance/suites" },
			{ label: "Scores", href: "/admin/governance/scores" },
		],
	},
	{
		title: "EU AI Act",
		description:
			"Conformity inventory, attached-model governance, and the GPAI model registry.",
		icon: ShieldAlert,
		href: "/admin/ai-act",
		permission: GlobalPermission.ReadPublishing,
		actionLabel: "Open Inventory",
		color: "text-indigo-500",
		feature: "ai_act",
		links: [
			{ label: "Inventory", href: "/admin/ai-act" },
			{ label: "Model Registry", href: "/admin/ai-act?tab=registry" },
		],
	},
	{
		title: "University",
		description: "Review drafts, create courses, and manage learning content.",
		icon: GraduationCap,
		href: "/learn/admin",
		permission: GlobalPermission.ReadCourses,
		alternatePermissions: [GlobalPermission.WriteCourses],
		actionLabel: "Open Courses",
		color: "text-sky-500",
		links: [
			{ label: "Catalog", href: "/learn" },
			{ label: "Authoring", href: "/learn/admin" },
		],
	},
	{
		title: "Home Layouts",
		description:
			"Publish the main home and optional defaults for profile templates.",
		icon: SlidersHorizontal,
		href: "/admin/home",
		permission: GlobalPermission.WriteLandingPage,
		actionLabel: "Edit Default Home",
		color: "text-orange-500",
	},
	{
		title: "Starter Profiles",
		description:
			"Curate profile images, introductions, bits, apps, and default homes.",
		icon: Users,
		href: "/admin/profiles",
		permission: GlobalPermission.ReadProfile,
		actionLabel: "Manage Profiles",
		color: "text-purple-500",
		links: [
			{ label: "Browse", href: "/admin/profiles" },
			{ label: "Manage", href: "/admin/profiles" },
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
	{
		title: "Process Graph",
		description:
			"Platform-wide map of app connections, observed call chains, and process notes.",
		icon: Waypoints,
		href: "/admin/connections",
		permission: GlobalPermission.Admin,
		actionLabel: "Open Process Graph",
		color: "text-teal-500",
	},
	{
		title: "Resources",
		description:
			"Database, cache, and object storage health, capacity, and throughput.",
		icon: HardDrive,
		href: "/admin/resources",
		permission: GlobalPermission.Admin,
		actionLabel: "Open",
		color: "text-fuchsia-500",
	},
	{
		title: "Logs & Observability",
		description:
			"Inspect API errors, drill into references, and verify cryptographic audit chains.",
		icon: Activity,
		href: "/admin/logs",
		permission: GlobalPermission.ReadLogs,
		actionLabel: "Open Control Tower",
		color: "text-red-500",
		links: [
			{ label: "Errors", href: "/admin/logs" },
			{ label: "Audit chain", href: "/admin/logs?tab=audit" },
		],
	},
	{
		title: "Telemetry",
		description:
			"Anonymous opt-in product metrics: events, active installs, and version adoption.",
		icon: Activity,
		href: "/admin/telemetry",
		permission: GlobalPermission.Admin,
		actionLabel: "Open Telemetry",
		color: "text-emerald-500",
		feature: "telemetry",
		links: [
			{ label: "Overview", href: "/admin/telemetry" },
			{ label: "Issues", href: "/admin/telemetry/issues" },
			{ label: "Traces", href: "/admin/telemetry/traces" },
			{ label: "Alerts", href: "/admin/telemetry/alerts" },
			{ label: "Prompt feedback", href: "/admin/telemetry/prompt-feedback" },
			{ label: "Query builder", href: "/admin/telemetry/query" },
			{ label: "Dashboards", href: "/admin/telemetry/dashboards" },
		],
	},
	{
		title: "Issues & Crashes",
		description:
			"Grouped crash and error reports with release health, symbolicated stacks, and triage.",
		icon: Bug,
		href: "/admin/telemetry/issues",
		permission: GlobalPermission.Admin,
		actionLabel: "Open Issues",
		color: "text-amber-500",
		feature: "telemetry",
	},
	{
		title: "FlowPilot feedback",
		description:
			"Assistant turns users rated, with the prompt, the model that ran it, and why it went wrong.",
		icon: MessageSquareHeart,
		href: "/admin/telemetry/prompt-feedback",
		permission: GlobalPermission.Admin,
		actionLabel: "Open Prompt Feedback",
		color: "text-lime-500",
		feature: "telemetry",
	},
	{
		title: "Traces & Performance",
		description:
			"Sampled distributed traces, span flamegraphs, and Core Web Vitals per path.",
		icon: GitBranch,
		href: "/admin/telemetry/traces",
		permission: GlobalPermission.Admin,
		actionLabel: "Open Traces",
		color: "text-violet-500",
		feature: "telemetry",
	},
	{
		title: "Alerts",
		description:
			"Threshold and anomaly rules over anonymous telemetry with an in-app alert inbox.",
		icon: BellRing,
		href: "/admin/telemetry/alerts",
		permission: GlobalPermission.Admin,
		actionLabel: "Open Alerts",
		color: "text-rose-500",
		feature: "telemetry",
	},
	{
		title: "Query builder",
		description:
			"Ad-hoc breakdowns over anonymous telemetry with saved queries and pinned dashboards.",
		icon: SlidersHorizontal,
		href: "/admin/telemetry/query",
		permission: GlobalPermission.Admin,
		actionLabel: "Open Query Builder",
		color: "text-cyan-500",
		feature: "telemetry",
		links: [
			{ label: "Query builder", href: "/admin/telemetry/query" },
			{ label: "Dashboards", href: "/admin/telemetry/dashboards" },
		],
	},
];

const GROUPS = [
	{
		name: "Content & publishing",
		titles: [
			"Bits & Models",
			"Packages",
			"Governance",
			"EU AI Act",
			"University",
			"Home Layouts",
			"Starter Profiles",
		],
	},
	{
		name: "People & access",
		titles: ["User Management", "Solutions", "Service Tokens"],
	},
	{
		name: "Operations & insights",
		titles: [
			"Resources",
			"Logs & Observability",
			"Process Graph",
			"Telemetry",
			"Issues & Crashes",
			"FlowPilot feedback",
			"Traces & Performance",
			"Alerts",
			"Query builder",
		],
	},
];

export function AdminDashboardNavigation({
	sections,
}: { sections: AdminSection[] }) {
	const { t } = useTranslation("admin");
	const [search, setSearch] = useState("");
	const query = search.trim().toLowerCase();
	const matching = sections.filter((section) =>
		`${section.title} ${section.description} ${section.links?.map((link) => link.label).join(" ") ?? ""}`
			.toLowerCase()
			.includes(query),
	);
	return (
		<section aria-labelledby="admin-manage-heading" className="space-y-5">
			<div className="flex flex-col justify-between gap-3 sm:flex-row sm:items-center">
				<div>
					<h2
						id="admin-manage-heading"
						className="text-lg font-semibold tracking-tight"
					>
						{t("dashboardWorkspace", "Your workspace")}
					</h2>
					<p className="mt-1 text-sm text-muted-foreground">
						{t(
							"dashboardWorkspaceDescription",
							"Find the tools available to your role.",
						)}
					</p>
				</div>
				<div className="relative w-full sm:w-72">
					<Search
						aria-hidden="true"
						className="absolute left-3 top-2.5 size-4 text-muted-foreground"
					/>
					<Input
						aria-label={t("dashboardSearchTools", "Find an admin tool")}
						placeholder={t("dashboardSearchTools", "Find an admin tool")}
						value={search}
						onChange={(event) => setSearch(event.target.value)}
						className="bg-card pl-9 pr-9"
					/>
					{search && (
						<Button
							type="button"
							variant="ghost"
							size="icon"
							className="absolute right-0 top-0 size-9"
							onClick={() => setSearch("")}
							aria-label={t("dashboardClearSearch", "Clear search")}
						>
							<X className="size-3.5" />
						</Button>
					)}
				</div>
			</div>
			{matching.length === 0 ? (
				<div className="rounded-xl border border-dashed p-8 text-center text-sm text-muted-foreground">
					{t(
						query ? "dashboardNoMatchingTools" : "dashboardNoTools",
						query
							? "No tools match your search."
							: "No admin tools are available for your role.",
					)}
				</div>
			) : (
				<div className="grid gap-6 lg:grid-cols-3">
					{GROUPS.map((group) => {
						const items = matching.filter((section) =>
							group.titles.includes(section.title),
						);
						if (!items.length) return null;
						return (
							<div key={group.name} className="min-w-0 space-y-3">
								<h3 className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
									{t(group.name, group.name)}
								</h3>
								<div className="overflow-hidden rounded-xl border bg-card shadow-xs divide-y divide-border/60">
									{items.map((section) => {
										const Icon = section.icon;
										return (
											<Link
												prefetch={false}
												key={section.title}
												href={section.href}
												className="group flex min-h-20 items-center gap-3 px-4 py-3.5 transition-colors hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
											>
												<span className="flex size-9 shrink-0 items-center justify-center rounded-lg border bg-background text-muted-foreground transition-colors group-hover:text-foreground">
													<Icon aria-hidden="true" className="size-4" />
												</span>
												<span className="min-w-0 flex-1">
													<span className="block text-sm font-medium">
														{t(section.title, section.title)}
													</span>
													<span className="mt-0.5 block text-xs leading-relaxed text-muted-foreground">
														{t(section.description, section.description)}
													</span>
												</span>
												<ArrowUpRight
													aria-hidden="true"
													className="size-3.5 shrink-0 text-muted-foreground/60 transition-colors group-hover:text-primary"
												/>
											</Link>
										);
									})}
								</div>
							</div>
						);
					})}
				</div>
			)}
		</section>
	);
}
