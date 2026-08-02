"use client";

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import {
	Activity,
	AlertTriangle,
	BellRing,
	BookOpen,
	Box,
	Bug,
	CheckCircle,
	Clock,
	Cpu,
	Download,
	GitBranch,
	GraduationCap,
	Key,
	Lightbulb,
	Lock,
	type LucideIcon,
	Package,
	Plus,
	RefreshCw,
	Save,
	Scale,
	Shield,
	ShieldAlert,
	SlidersHorizontal,
	UserCog,
	Users,
	Waypoints,
} from "lucide-react";
import Link from "next/link";
import { useEffect, useMemo, useState } from "react";
import {
	Bar,
	BarChart,
	CartesianGrid,
	Cell,
	ComposedChart,
	Line,
	Pie,
	PieChart,
	XAxis,
	YAxis,
} from "recharts";
import { useFeatures } from "../../../hooks/use-features";
import { useInvoke } from "../../../hooks/use-invoke";
import { GlobalPermission } from "../../../lib/permission/global-permission";
import type { ISolutionListResponse } from "../../../lib/schema/solution/solution";
import type {
	IAdminAppUsage,
	IAdminPaginated,
	IAdminTechnicalUserUsage,
	IAdminUsageAlert,
	IAdminUsageInvocation,
	IAdminUsageOverview,
	IAppUsageLimits,
	IUsageLimitPeriod,
	IUsageReconciliationResult,
} from "../../../lib/schema/usage";
import type { AdminEnsureWasmArtifactsResponse } from "../../../lib/schema/wasm";
import { useBackend } from "../../../state/backend-state";
import type { IProfile } from "../../../types";
import {
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	type ChartConfig,
	ChartContainer,
	ChartTooltip,
	ChartTooltipContent,
	Input,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Skeleton,
	UserProfileLink,
} from "../../ui";
import { DashboardChainWidget, DashboardErrorWidget } from "./logs";
import { DashboardTelemetryAlertsWidget } from "./telemetry/alerts-dashboard-widget";
import { DashboardTelemetryWidget } from "./telemetry/dashboard-telemetry-widget";
import { DashboardTelemetryIssuesWidget } from "./telemetry/issues-dashboard-widget";
import { DashboardTelemetryTracesWidget } from "./telemetry/traces-dashboard-widget";

const PERIODS: IUsageLimitPeriod[] = ["weekly", "monthly", "yearly"];

const activityChartConfig = {
	aiCalls: {
		label: "AI calls",
		color: "var(--chart-1)",
	},
	executions: {
		label: "Executions",
		color: "var(--chart-2)",
	},
	activeUsers: {
		label: "Active users",
		color: "var(--chart-3)",
	},
	newUsers: {
		label: "New users",
		color: "var(--chart-4)",
	},
} satisfies ChartConfig;

const spendChartConfig = {
	costDollars: {
		label: "Cost",
		color: "var(--chart-1)",
	},
	tokens: {
		label: "Tokens",
		color: "var(--chart-5)",
	},
} satisfies ChartConfig;

const appActivityChartConfig = {
	aiCalls: {
		label: "AI calls",
		color: "var(--chart-1)",
	},
	executions: {
		label: "Executions",
		color: "var(--chart-2)",
	},
} satisfies ChartConfig;

const modelChartConfig = {
	invocations: {
		label: "Calls",
		color: "var(--chart-3)",
	},
	tokens: {
		label: "Tokens",
		color: "var(--chart-5)",
	},
} satisfies ChartConfig;

const spendMixChartConfig = {
	llm: {
		label: "LLM",
		color: "var(--chart-1)",
	},
	embedding: {
		label: "Embeddings",
		color: "var(--chart-2)",
	},
} satisfies ChartConfig;

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
		<Card className="min-w-0 transition-colors hover:border-primary/40">
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
	alternatePermissions?: GlobalPermission[];
	actionLabel: string;
	color: string;
	links?: { label: string; href: string }[];
	/** When set, the section is only shown if the named hub feature is on. */
	feature?: string;
}

const ADMIN_SECTIONS: AdminSection[] = [
	{
		title: "Bits & Models",
		description:
			"Add hosted LLMs, manage existing bits, and edit model metadata.",
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
			{ label: "Review Queue", href: "/admin/governance/requests" },
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

function emptyLimits(): IAppUsageLimits {
	const window = {
		costMicroDollars: null,
		tokenLimit: null,
		enabled: true,
		hard: true,
		warningThresholdPercent: 80,
	};
	return {
		weekly: { ...window },
		monthly: { ...window },
		yearly: { ...window },
	};
}

function formatCost(microDollars: number) {
	return new Intl.NumberFormat(undefined, {
		style: "currency",
		currency: "USD",
		maximumFractionDigits: 4,
	}).format(microDollars / 1_000_000);
}

function formatDollars(value: number | null) {
	if (value === null) return "n/a";
	return new Intl.NumberFormat(undefined, {
		style: "currency",
		currency: "USD",
		maximumFractionDigits: 4,
	}).format(value);
}

function formatCount(value: number) {
	return new Intl.NumberFormat().format(value);
}

function formatCompactCount(value: number) {
	return new Intl.NumberFormat(undefined, {
		notation: "compact",
		maximumFractionDigits: 1,
	}).format(value);
}

function formatDuration(ms: number | null) {
	if (ms === null) return "n/a";
	if (ms < 1000) return `${Math.round(ms)} ms`;
	return `${(ms / 1000).toFixed(1)} s`;
}

function formatPercent(value: number) {
	if (!Number.isFinite(value)) return "0%";
	return `${Math.round(value)}%`;
}

function chartTick(value: string | number) {
	const numeric = Number(value);
	return Number.isFinite(numeric) ? formatCompactCount(numeric) : String(value);
}

function currencyTick(value: string | number) {
	const numeric = Number(value);
	if (!Number.isFinite(numeric)) return String(value);
	return `$${formatCompactCount(numeric)}`;
}

function dollarValue(microDollars: number) {
	return microDollars / 1_000_000;
}

function truncateChartLabel(value: string, maxLength = 18) {
	return value.length > maxLength
		? `${value.slice(0, maxLength - 3)}...`
		: value;
}

function periodTitle(period: IUsageLimitPeriod) {
	return period[0].toUpperCase() + period.slice(1);
}

function costInputValue(
	limits: IAppUsageLimits | null,
	period: IUsageLimitPeriod,
) {
	const value = limits?.[period]?.costMicroDollars;
	return value == null ? "" : String(value / 1_000_000);
}

function tokenInputValue(
	limits: IAppUsageLimits | null,
	period: IUsageLimitPeriod,
) {
	const value = limits?.[period]?.tokenLimit;
	return value == null ? "" : String(value);
}

function toMicroDollars(value: string) {
	const parsed = Number(value);
	if (!Number.isFinite(parsed) || parsed < 0) return null;
	return Math.round(parsed * 1_000_000);
}

function toTokenLimit(value: string) {
	const parsed = Number(value);
	if (!Number.isFinite(parsed) || parsed < 0) return null;
	return Math.floor(parsed);
}

function EmptyChart({ label }: { label: string }) {
	return (
		<div className="flex h-64 items-center justify-center rounded-lg border border-dashed text-sm text-muted-foreground">
			{label}
		</div>
	);
}

function UsageHealthSummary({
	overview,
	loading,
	period,
}: {
	overview: IAdminUsageOverview | undefined;
	loading: boolean;
	period: IUsageLimitPeriod;
}) {
	const stats = overview?.userStats;
	const totals = overview?.totals;

	const groups = [
		{
			title: "Audience",
			items: [
				["DAU", formatCount(stats?.activeUsersDaily ?? 0)],
				["WAU", formatCount(stats?.activeUsersWeekly ?? 0)],
				["MAU", formatCount(stats?.activeUsersMonthly ?? 0)],
			],
		},
		{
			title: "Growth",
			items: [
				["Today", formatCount(stats?.newUsersToday ?? 0)],
				["7d", formatCount(stats?.newUsersWeekly ?? 0)],
				["30d", formatCount(stats?.newUsersMonthly ?? 0)],
			],
		},
		{
			title: "Workload",
			items: [
				[
					"AI",
					formatCount(
						(totals?.llmInvocations ?? 0) + (totals?.embeddingInvocations ?? 0),
					),
				],
				["Runs", formatCount(totals?.executions ?? 0)],
				["Apps", formatCount(stats?.activeAppsMonthly ?? 0)],
			],
		},
		{
			title: "Efficiency",
			items: [
				["Spend", formatCost(totals?.totalPrice ?? 0)],
				["/ MAU", formatDollars(stats?.averageCostPerActiveUser ?? null)],
				["Runtime", formatDuration(totals?.averageExecutionMs ?? null)],
			],
		},
	];

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">
					{periodTitle(period)} Health
				</CardTitle>
				<CardDescription>
					Users, growth, workload, and spend in one operational view.
				</CardDescription>
			</CardHeader>
			<CardContent>
				{loading ? (
					<Skeleton className="h-64 w-full" />
				) : (
					<div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-1">
						{groups.map((group) => (
							<div key={group.title} className="rounded-lg border p-3">
								<div className="mb-3 text-sm font-medium">{group.title}</div>
								<div className="grid grid-cols-3 gap-2">
									{group.items.map(([label, value]) => (
										<div key={label} className="min-w-0">
											<div className="truncate text-[11px] text-muted-foreground">
												{label}
											</div>
											<div className="truncate text-sm font-semibold">
												{value}
											</div>
										</div>
									))}
								</div>
							</div>
						))}
					</div>
				)}
			</CardContent>
		</Card>
	);
}

function ActivityTrendChart({
	overview,
	loading,
}: {
	overview: IAdminUsageOverview | undefined;
	loading: boolean;
}) {
	const data = useMemo(
		() =>
			overview?.trend.map((point) => ({
				label: point.label,
				aiCalls: point.aiInvocations,
				executions: point.executions,
				activeUsers: point.activeUsers,
				newUsers: point.newUsers,
			})) ?? [],
		[overview?.trend],
	);
	const hasData = data.some(
		(point) =>
			point.aiCalls || point.executions || point.activeUsers || point.newUsers,
	);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">Activity Over Time</CardTitle>
				<CardDescription>
					AI calls, app executions, active users, and signups.
				</CardDescription>
			</CardHeader>
			<CardContent>
				{loading ? (
					<Skeleton className="h-80 w-full" />
				) : !hasData ? (
					<EmptyChart label="No activity recorded for this period." />
				) : (
					<ChartContainer config={activityChartConfig} className="h-80 w-full">
						<ComposedChart
							data={data}
							margin={{ top: 12, right: 12, left: 0, bottom: 0 }}
						>
							<CartesianGrid strokeDasharray="3 3" vertical={false} />
							<XAxis
								dataKey="label"
								tickLine={false}
								axisLine={false}
								tick={{ fontSize: 11 }}
								minTickGap={24}
							/>
							<YAxis
								tickLine={false}
								axisLine={false}
								tick={{ fontSize: 11 }}
								width={36}
								tickFormatter={chartTick}
								allowDecimals={false}
							/>
							<ChartTooltip
								cursor={{ stroke: "var(--border)", strokeDasharray: "3 3" }}
								content={<ChartTooltipContent indicator="dot" />}
							/>
							<Bar
								dataKey="aiCalls"
								fill="var(--color-aiCalls)"
								radius={[3, 3, 0, 0]}
							/>
							<Bar
								dataKey="executions"
								fill="var(--color-executions)"
								radius={[3, 3, 0, 0]}
							/>
							<Line
								type="monotone"
								dataKey="activeUsers"
								stroke="var(--color-activeUsers)"
								strokeWidth={2}
								dot={false}
							/>
							<Line
								type="monotone"
								dataKey="newUsers"
								stroke="var(--color-newUsers)"
								strokeWidth={2}
								dot={false}
							/>
						</ComposedChart>
					</ChartContainer>
				)}
			</CardContent>
		</Card>
	);
}

function SpendTokenTrendChart({
	overview,
	loading,
}: {
	overview: IAdminUsageOverview | undefined;
	loading: boolean;
}) {
	const data = useMemo(
		() =>
			overview?.trend.map((point) => ({
				label: point.label,
				costDollars: dollarValue(point.cost),
				tokens: point.tokens,
			})) ?? [],
		[overview?.trend],
	);
	const hasData = data.some((point) => point.costDollars || point.tokens);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">Spend & Tokens</CardTitle>
				<CardDescription>
					Remote model cost against token volume.
				</CardDescription>
			</CardHeader>
			<CardContent>
				{loading ? (
					<Skeleton className="h-72 w-full" />
				) : !hasData ? (
					<EmptyChart label="No remote model spend for this period." />
				) : (
					<ChartContainer config={spendChartConfig} className="h-72 w-full">
						<ComposedChart
							data={data}
							margin={{ top: 12, right: 8, left: 0, bottom: 0 }}
						>
							<CartesianGrid strokeDasharray="3 3" vertical={false} />
							<XAxis
								dataKey="label"
								tickLine={false}
								axisLine={false}
								tick={{ fontSize: 11 }}
								minTickGap={24}
							/>
							<YAxis
								yAxisId="cost"
								tickLine={false}
								axisLine={false}
								tick={{ fontSize: 11 }}
								width={42}
								tickFormatter={currencyTick}
							/>
							<YAxis
								yAxisId="tokens"
								orientation="right"
								tickLine={false}
								axisLine={false}
								tick={{ fontSize: 11 }}
								width={42}
								tickFormatter={chartTick}
							/>
							<ChartTooltip
								cursor={{ stroke: "var(--border)", strokeDasharray: "3 3" }}
								content={<ChartTooltipContent indicator="dot" />}
							/>
							<Bar
								yAxisId="tokens"
								dataKey="tokens"
								fill="var(--color-tokens)"
								radius={[3, 3, 0, 0]}
							/>
							<Line
								yAxisId="cost"
								type="monotone"
								dataKey="costDollars"
								stroke="var(--color-costDollars)"
								strokeWidth={2}
								dot={false}
							/>
						</ComposedChart>
					</ChartContainer>
				)}
			</CardContent>
		</Card>
	);
}

function SpendMixChart({
	overview,
	loading,
}: {
	overview: IAdminUsageOverview | undefined;
	loading: boolean;
}) {
	const data = useMemo(
		() => [
			{
				key: "llm",
				name: "LLM",
				value: overview?.totals.llmPrice ?? 0,
			},
			{
				key: "embedding",
				name: "Embeddings",
				value: overview?.totals.embeddingPrice ?? 0,
			},
		],
		[overview?.totals.embeddingPrice, overview?.totals.llmPrice],
	);
	const total = data.reduce((sum, item) => sum + item.value, 0);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">Spend Mix</CardTitle>
				<CardDescription>LLM cost versus embedding cost.</CardDescription>
			</CardHeader>
			<CardContent>
				{loading ? (
					<Skeleton className="h-72 w-full" />
				) : total === 0 ? (
					<EmptyChart label="No paid remote usage yet." />
				) : (
					<div className="grid gap-4 md:grid-cols-[1fr_auto] lg:grid-cols-1 xl:grid-cols-[1fr_auto]">
						<ChartContainer
							config={spendMixChartConfig}
							className="h-56 w-full"
						>
							<PieChart>
								<ChartTooltip
									content={<ChartTooltipContent nameKey="key" hideLabel />}
								/>
								<Pie
									data={data}
									dataKey="value"
									nameKey="key"
									innerRadius={54}
									outerRadius={86}
									paddingAngle={2}
								>
									{data.map((entry) => (
										<Cell key={entry.key} fill={`var(--color-${entry.key})`} />
									))}
								</Pie>
							</PieChart>
						</ChartContainer>
						<div className="space-y-3 text-sm">
							{data.map((item) => (
								<div key={item.key} className="min-w-36">
									<div className="flex items-center justify-between gap-3">
										<div className="flex items-center gap-2">
											<span
												className="h-2.5 w-2.5 rounded-sm"
												style={{ background: `var(--color-${item.key})` }}
											/>
											<span>{item.name}</span>
										</div>
										<span className="font-medium">
											{formatCost(item.value)}
										</span>
									</div>
									<div className="text-xs text-muted-foreground">
										{formatPercent((item.value / total) * 100)}
									</div>
								</div>
							))}
						</div>
					</div>
				)}
			</CardContent>
		</Card>
	);
}

function TechnicalUsers({
	overview,
	loading,
	profile,
	period,
}: {
	overview: IAdminUsageOverview | undefined;
	loading: boolean;
	profile: IProfile | undefined;
	period: IUsageLimitPeriod;
}) {
	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">Technical Users</CardTitle>
				<CardDescription>
					API keys with the highest tracked usage.
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-2">
				{loading && <Skeleton className="h-36 w-full" />}
				{overview?.technicalUsers.map((technicalUser) => {
					return (
						<div
							key={technicalUser.technicalUserId}
							className="space-y-3 rounded-md border p-3 text-sm"
						>
							<div className="grid grid-cols-[1fr_auto] gap-3">
								<div className="min-w-0">
									<div className="truncate font-medium">
										{technicalUser.name ?? technicalUser.technicalUserId}
									</div>
									<div className="flex min-w-0 items-center gap-1 text-xs text-muted-foreground">
										<UserProfileLink
											userId={technicalUser.creatorUserId}
											name={technicalUser.creatorDisplayName}
											email={technicalUser.creatorEmail}
											fallbackLabel="Unknown owner"
											className="min-w-0"
											muted
										/>
										{(technicalUser.appName || technicalUser.appId) && (
											<>
												<span className="shrink-0">-</span>
												<span className="truncate">
													{technicalUser.appName ?? technicalUser.appId}
												</span>
											</>
										)}
									</div>
								</div>
								<div className="text-right">
									<div className="font-medium">
										{formatCost(technicalUser.totalPrice)}
									</div>
									<div className="text-xs text-muted-foreground">
										{formatCount(technicalUser.totalTokens)} tokens
									</div>
								</div>
							</div>
							{profile && technicalUser.appId && (
								<TechnicalUserLimitEditor
									technicalUser={technicalUser}
									period={period}
									profile={profile}
								/>
							)}
						</div>
					);
				})}
				{overview && overview.technicalUsers.length === 0 && (
					<div className="rounded-md border p-4 text-sm text-muted-foreground">
						No API-key usage for this period.
					</div>
				)}
			</CardContent>
		</Card>
	);
}

function TechnicalUserLimitEditor({
	technicalUser,
	period,
	profile,
}: {
	technicalUser: IAdminTechnicalUserUsage;
	period: IUsageLimitPeriod;
	profile: IProfile;
}) {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const [cost, setCost] = useState(
		costInputValue(technicalUser.limits, period),
	);
	const [tokens, setTokens] = useState(
		tokenInputValue(technicalUser.limits, period),
	);

	useEffect(() => {
		setCost(costInputValue(technicalUser.limits, period));
		setTokens(tokenInputValue(technicalUser.limits, period));
	}, [technicalUser.limits, period]);

	const mutation = useMutation({
		mutationFn: async () => {
			if (!technicalUser.appId) throw new Error("Missing app id");
			const next = technicalUser.limits ?? emptyLimits();
			return backend.apiState.put<IAppUsageLimits>(
				profile,
				`admin/usage/apps/${technicalUser.appId}/technical-users/${technicalUser.technicalUserId}/limits`,
				{
					...next,
					[period]: {
						...next[period],
						costMicroDollars: cost.trim() ? toMicroDollars(cost) : null,
						tokenLimit: tokens.trim() ? toTokenLimit(tokens) : null,
					},
				},
			);
		},
		onSuccess: async () => {
			await queryClient.invalidateQueries({ queryKey: ["admin", "usage"] });
		},
	});

	return (
		<div className="grid gap-2 border-t pt-3 sm:grid-cols-[1fr_1fr_auto]">
			<Input
				inputMode="decimal"
				placeholder="$ limit"
				value={cost}
				onChange={(event) => setCost(event.target.value)}
				className="h-8"
			/>
			<Input
				inputMode="numeric"
				placeholder="Token limit"
				value={tokens}
				onChange={(event) => setTokens(event.target.value)}
				className="h-8"
			/>
			<Button
				size="icon"
				variant="outline"
				disabled={mutation.isPending}
				onClick={() => mutation.mutate()}
				aria-label="Save technical user usage limit"
			>
				<Save className="h-4 w-4" />
			</Button>
		</div>
	);
}

function PowerUsers({
	overview,
	loading,
}: {
	overview: IAdminUsageOverview | undefined;
	loading: boolean;
}) {
	const maxInteractions = Math.max(
		1,
		...(overview?.powerUsers.map((user) => user.totalInteractions) ?? []),
	);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">Power Users</CardTitle>
				<CardDescription>
					Highest activity across the last 30 days.
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-2">
				{loading && <Skeleton className="h-36 w-full" />}
				{overview?.powerUsers.map((user) => (
					<div
						key={user.userId}
						className="grid grid-cols-[1fr_auto] gap-3 rounded-md border p-3 text-sm"
					>
						<div className="min-w-0">
							<div className="truncate font-medium">
								{user.displayName ?? user.email ?? user.userId}
							</div>
							<div className="truncate text-xs text-muted-foreground">
								{formatCount(user.aiInvocations)} AI,{" "}
								{formatCount(user.executions)} runs, {user.activeDays} active
								days
							</div>
						</div>
						<div className="text-right">
							<div className="font-medium">
								{formatCount(user.totalInteractions)}
							</div>
							<div className="text-xs text-muted-foreground">
								{formatCost(user.totalPrice)}
							</div>
						</div>
						<div className="col-span-2 h-1.5 overflow-hidden rounded-full bg-muted">
							<div
								className="h-full rounded-full bg-primary"
								style={{
									width: `${Math.max(
										4,
										(user.totalInteractions / maxInteractions) * 100,
									)}%`,
								}}
							/>
						</div>
					</div>
				))}
				{overview && overview.powerUsers.length === 0 && (
					<div className="rounded-md border p-4 text-sm text-muted-foreground">
						No power users in the last 30 days.
					</div>
				)}
			</CardContent>
		</Card>
	);
}

function TopAppsActivityChart({
	overview,
	loading,
}: {
	overview: IAdminUsageOverview | undefined;
	loading: boolean;
}) {
	const data = useMemo(
		() =>
			(overview?.apps ?? []).slice(0, 8).map((app, index) => ({
				name: truncateChartLabel(
					app.appName ?? app.appId ?? `App ${index + 1}`,
				),
				fullName: app.appName ?? app.appId ?? "Unknown app",
				aiCalls: app.llmInvocations + app.embeddingInvocations,
				executions: app.executions,
				cost: app.totalPrice,
			})),
		[overview?.apps],
	);
	const hasData = data.some((item) => item.aiCalls || item.executions);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">Top Apps by Activity</CardTitle>
				<CardDescription>Remote AI calls and app executions.</CardDescription>
			</CardHeader>
			<CardContent>
				{loading ? (
					<Skeleton className="h-72 w-full" />
				) : !hasData ? (
					<EmptyChart label="No app activity for this period." />
				) : (
					<ChartContainer
						config={appActivityChartConfig}
						className="h-72 w-full"
					>
						<BarChart
							data={data}
							layout="vertical"
							margin={{ top: 8, right: 16, left: 8, bottom: 0 }}
						>
							<CartesianGrid strokeDasharray="3 3" horizontal={false} />
							<XAxis
								type="number"
								tickLine={false}
								axisLine={false}
								tickFormatter={chartTick}
								allowDecimals={false}
							/>
							<YAxis
								type="category"
								dataKey="name"
								tickLine={false}
								axisLine={false}
								width={112}
								tick={{ fontSize: 11 }}
							/>
							<ChartTooltip
								cursor={{ fill: "var(--muted)" }}
								content={<ChartTooltipContent indicator="dot" />}
							/>
							<Bar
								dataKey="aiCalls"
								stackId="activity"
								fill="var(--color-aiCalls)"
								radius={[0, 3, 3, 0]}
							/>
							<Bar
								dataKey="executions"
								stackId="activity"
								fill="var(--color-executions)"
								radius={[0, 3, 3, 0]}
							/>
						</BarChart>
					</ChartContainer>
				)}
			</CardContent>
		</Card>
	);
}

function TopModelsChart({
	overview,
	loading,
}: {
	overview: IAdminUsageOverview | undefined;
	loading: boolean;
}) {
	const data = useMemo(
		() =>
			(overview?.models ?? []).slice(0, 8).map((model) => ({
				name: truncateChartLabel(model.modelId, 22),
				fullName: model.modelId,
				invocations: model.invocations,
				tokens: model.tokens,
				cost: model.price,
				kind: model.kind,
			})),
		[overview?.models],
	);
	const hasData = data.some((item) => item.invocations || item.tokens);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">Remote Model Mix</CardTitle>
				<CardDescription>
					Which remote models are carrying traffic.
				</CardDescription>
			</CardHeader>
			<CardContent>
				{loading ? (
					<Skeleton className="h-72 w-full" />
				) : !hasData ? (
					<EmptyChart label="No remote model usage for this period." />
				) : (
					<div className="space-y-4">
						<ChartContainer config={modelChartConfig} className="h-64 w-full">
							<BarChart
								data={data}
								layout="vertical"
								margin={{ top: 8, right: 16, left: 8, bottom: 0 }}
							>
								<CartesianGrid strokeDasharray="3 3" horizontal={false} />
								<XAxis
									type="number"
									tickLine={false}
									axisLine={false}
									tickFormatter={chartTick}
									allowDecimals={false}
								/>
								<YAxis
									type="category"
									dataKey="name"
									tickLine={false}
									axisLine={false}
									width={132}
									tick={{ fontSize: 11 }}
								/>
								<ChartTooltip
									cursor={{ fill: "var(--muted)" }}
									content={<ChartTooltipContent indicator="dot" />}
								/>
								<Bar
									dataKey="invocations"
									fill="var(--color-invocations)"
									radius={[0, 3, 3, 0]}
								/>
							</BarChart>
						</ChartContainer>
						<div className="grid gap-2 text-xs sm:grid-cols-2">
							{data.slice(0, 4).map((model) => (
								<div
									key={model.fullName}
									className="rounded-md border px-3 py-2"
								>
									<div className="truncate font-medium">{model.fullName}</div>
									<div className="text-muted-foreground">
										{model.kind} - {formatCount(model.tokens)} tokens -{" "}
										{formatCost(model.cost)}
									</div>
								</div>
							))}
						</div>
					</div>
				)}
			</CardContent>
		</Card>
	);
}

function LimitUtilization({
	overview,
	loading,
	period,
}: {
	overview: IAdminUsageOverview | undefined;
	loading: boolean;
	period: IUsageLimitPeriod;
}) {
	const rows = useMemo(
		() =>
			(overview?.apps ?? [])
				.map((app) => {
					const window = app.limits?.[period];
					if (!window?.enabled) return null;
					const costPercent =
						window.costMicroDollars && window.costMicroDollars > 0
							? (app.totalPrice / window.costMicroDollars) * 100
							: null;
					const tokenPercent =
						window.tokenLimit && window.tokenLimit > 0
							? (app.totalTokens / window.tokenLimit) * 100
							: null;
					const utilization = Math.max(costPercent ?? 0, tokenPercent ?? 0);
					if (!costPercent && !tokenPercent) return null;
					return {
						id: app.appId ?? app.appName ?? "unknown",
						name: app.appName ?? app.appId ?? "Unknown app",
						utilization,
						costPercent,
						tokenPercent,
						hard: window.hard,
					};
				})
				.filter((row): row is NonNullable<typeof row> => Boolean(row))
				.sort((a, b) => b.utilization - a.utilization)
				.slice(0, 7),
		[overview?.apps, period],
	);

	return (
		<Card>
			<CardHeader>
				<CardTitle className="text-base">Limit Utilization</CardTitle>
				<CardDescription>
					Apps closest to their {period} guardrails.
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-3">
				{loading && <Skeleton className="h-64 w-full" />}
				{!loading &&
					rows.map((row) => {
						const color =
							row.utilization >= 100
								? "hsl(var(--destructive))"
								: row.utilization >= 80
									? "hsl(var(--chart-4))"
									: "hsl(var(--chart-2))";
						return (
							<div key={row.id} className="space-y-1.5">
								<div className="flex items-center justify-between gap-3 text-sm">
									<div className="min-w-0 truncate font-medium">{row.name}</div>
									<div className="shrink-0 text-xs text-muted-foreground">
										{formatPercent(row.utilization)}
										{row.hard ? " hard" : " soft"}
									</div>
								</div>
								<div className="h-2 overflow-hidden rounded-full bg-muted">
									<div
										className="h-full rounded-full"
										style={{
											width: `${Math.min(100, Math.max(3, row.utilization))}%`,
											backgroundColor: color,
										}}
									/>
								</div>
								<div className="flex gap-3 text-[11px] text-muted-foreground">
									<span>
										Cost{" "}
										{row.costPercent === null
											? "n/a"
											: formatPercent(row.costPercent)}
									</span>
									<span>
										Tokens{" "}
										{row.tokenPercent === null
											? "n/a"
											: formatPercent(row.tokenPercent)}
									</span>
								</div>
							</div>
						);
					})}
				{!loading && rows.length === 0 && (
					<div className="rounded-md border p-4 text-sm text-muted-foreground">
						No limits configured for active apps in this period.
					</div>
				)}
			</CardContent>
		</Card>
	);
}

function LimitEditor({
	app,
	period,
	profile,
}: {
	app: IAdminAppUsage;
	period: IUsageLimitPeriod;
	profile: IProfile;
}) {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const [cost, setCost] = useState(costInputValue(app.limits, period));
	const [tokens, setTokens] = useState(tokenInputValue(app.limits, period));

	useEffect(() => {
		setCost(costInputValue(app.limits, period));
		setTokens(tokenInputValue(app.limits, period));
	}, [app.limits, period]);

	const mutation = useMutation({
		mutationFn: async () => {
			if (!app.appId) throw new Error("Missing app id");
			const next = app.limits ?? emptyLimits();
			return backend.apiState.put<IAppUsageLimits>(
				profile,
				`admin/usage/apps/${app.appId}/limits`,
				{
					...next,
					[period]: {
						...next[period],
						costMicroDollars: cost.trim() ? toMicroDollars(cost) : null,
						tokenLimit: tokens.trim() ? toTokenLimit(tokens) : null,
					},
				},
			);
		},
		onSuccess: async () => {
			await queryClient.invalidateQueries({ queryKey: ["admin", "usage"] });
		},
	});

	if (!app.appId) {
		return (
			<span className="text-xs text-muted-foreground">No app context</span>
		);
	}

	return (
		<div className="grid gap-2 sm:grid-cols-[1fr_1fr_auto]">
			<Input
				inputMode="decimal"
				placeholder="$ limit"
				value={cost}
				onChange={(event) => setCost(event.target.value)}
				className="h-8"
			/>
			<Input
				inputMode="numeric"
				placeholder="Token limit"
				value={tokens}
				onChange={(event) => setTokens(event.target.value)}
				className="h-8"
			/>
			<Button
				size="icon"
				variant="outline"
				disabled={mutation.isPending}
				onClick={() => mutation.mutate()}
				aria-label="Save usage limit"
			>
				<Save className="h-4 w-4" />
			</Button>
		</div>
	);
}

function ManualLimitEditor({
	profile,
	period,
}: {
	profile: IProfile;
	period: IUsageLimitPeriod;
}) {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const [appId, setAppId] = useState("");
	const [cost, setCost] = useState("");
	const [tokens, setTokens] = useState("");

	const mutation = useMutation({
		mutationFn: async () => {
			const existing = await backend.apiState.get<IAppUsageLimits>(
				profile,
				`admin/usage/apps/${appId.trim()}/limits`,
			);
			return backend.apiState.put<IAppUsageLimits>(
				profile,
				`admin/usage/apps/${appId.trim()}/limits`,
				{
					...existing,
					[period]: {
						...existing[period],
						costMicroDollars: cost.trim() ? toMicroDollars(cost) : null,
						tokenLimit: tokens.trim() ? toTokenLimit(tokens) : null,
					},
				},
			);
		},
		onSuccess: async () => {
			setCost("");
			setTokens("");
			await queryClient.invalidateQueries({ queryKey: ["admin", "usage"] });
		},
	});

	return (
		<div className="grid gap-3 border-t pt-4 sm:grid-cols-[1.5fr_1fr_1fr_auto]">
			<div className="space-y-1">
				<Label htmlFor="usage-limit-app-id">App ID</Label>
				<Input
					id="usage-limit-app-id"
					value={appId}
					onChange={(event) => setAppId(event.target.value)}
					placeholder="app_..."
				/>
			</div>
			<div className="space-y-1">
				<Label htmlFor="usage-limit-cost">Cost limit</Label>
				<Input
					id="usage-limit-cost"
					inputMode="decimal"
					value={cost}
					onChange={(event) => setCost(event.target.value)}
					placeholder="$"
				/>
			</div>
			<div className="space-y-1">
				<Label htmlFor="usage-limit-tokens">Token limit</Label>
				<Input
					id="usage-limit-tokens"
					inputMode="numeric"
					value={tokens}
					onChange={(event) => setTokens(event.target.value)}
					placeholder="tokens"
				/>
			</div>
			<div className="flex items-end">
				<Button
					className="w-full sm:w-auto"
					disabled={!appId.trim() || mutation.isPending}
					onClick={() => mutation.mutate()}
				>
					<Save className="mr-2 h-4 w-4" />
					Save
				</Button>
			</div>
		</div>
	);
}

function UsageOperations({
	profile,
	period,
}: {
	profile: IProfile;
	period: IUsageLimitPeriod;
}) {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const alerts = useQuery<IAdminPaginated<IAdminUsageAlert>>({
		queryKey: ["admin", "usage", "alerts"],
		queryFn: () =>
			backend.apiState.get<IAdminPaginated<IAdminUsageAlert>>(
				profile,
				"admin/usage/alerts?page_size=5",
			),
	});
	const invocations = useQuery<IAdminPaginated<IAdminUsageInvocation>>({
		queryKey: ["admin", "usage", "invocations", period],
		queryFn: () =>
			backend.apiState.get<IAdminPaginated<IAdminUsageInvocation>>(
				profile,
				`admin/usage/invocations?period=${period}&page_size=8`,
			),
	});
	const reconcile = useMutation({
		mutationFn: () =>
			backend.apiState.post<IUsageReconciliationResult>(
				profile,
				"admin/usage/reconcile?older_than_minutes=30",
				{},
			),
		onSuccess: async () => {
			await queryClient.invalidateQueries({ queryKey: ["admin", "usage"] });
		},
	});
	const acknowledge = useMutation({
		mutationFn: (alertId: string) =>
			backend.apiState.post<IAdminUsageAlert>(
				profile,
				`admin/usage/alerts/${alertId}/ack`,
				{},
			),
		onSuccess: async () => {
			await queryClient.invalidateQueries({
				queryKey: ["admin", "usage", "alerts"],
			});
		},
	});

	return (
		<div className="grid gap-4 lg:grid-cols-2">
			<Card>
				<CardHeader className="flex flex-row items-start justify-between gap-3">
					<div>
						<CardTitle className="text-base">Usage Ledger</CardTitle>
						<CardDescription>
							Recent provider calls, including pending and unknown usage.
						</CardDescription>
					</div>
					<Button
						size="sm"
						variant="outline"
						disabled={reconcile.isPending}
						onClick={() => reconcile.mutate()}
					>
						<RefreshCw className="mr-2 h-4 w-4" />
						Reconcile
					</Button>
				</CardHeader>
				<CardContent className="space-y-2">
					{invocations.isLoading && <Skeleton className="h-32 w-full" />}
					{invocations.data?.items.map((item) => (
						<div
							key={item.id}
							className="grid grid-cols-[1fr_auto] gap-3 rounded-md border p-3 text-sm"
						>
							<div className="min-w-0">
								<div className="truncate font-medium">
									{item.modelId ?? item.kind}
								</div>
								<div className="truncate text-xs text-muted-foreground">
									{item.provider ?? "provider"} - {item.status} -{" "}
									{item.appId ?? "no app"}
									{item.technicalUserId ? ` - key ${item.technicalUserId}` : ""}
								</div>
							</div>
							<div className="text-right">
								<div className="font-medium">
									{formatCost(
										item.costMicroDollars || item.estimatedCostMicroDollars,
									)}
								</div>
								<div className="text-xs text-muted-foreground">
									{formatCount(
										item.inputTokens +
											item.outputTokens +
											item.embeddingTokens || item.estimatedTokens,
									)}{" "}
									tokens
								</div>
							</div>
						</div>
					))}
					{invocations.data?.items.length === 0 && (
						<div className="rounded-md border p-4 text-sm text-muted-foreground">
							No ledger entries for this period.
						</div>
					)}
				</CardContent>
			</Card>
			<Card>
				<CardHeader>
					<CardTitle className="text-base">Usage Alerts</CardTitle>
					<CardDescription>
						Limit warnings, hard blocks, and cost anomalies.
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-2">
					{alerts.isLoading && <Skeleton className="h-32 w-full" />}
					{alerts.data?.items.map((alert) => (
						<div
							key={alert.id}
							className="grid grid-cols-[1fr_auto] gap-3 rounded-md border p-3 text-sm"
						>
							<div className="min-w-0">
								<div className="flex items-center gap-2 font-medium">
									<AlertTriangle className="h-4 w-4 text-amber-500" />
									<span className="truncate">{alert.message}</span>
								</div>
								<div className="truncate text-xs text-muted-foreground">
									{alert.severity} - {alert.period ?? "period"} -{" "}
									{alert.appId ?? "no app"}
								</div>
							</div>
							<Button
								size="sm"
								variant="outline"
								disabled={
									Boolean(alert.acknowledgedAt) || acknowledge.isPending
								}
								onClick={() => acknowledge.mutate(alert.id)}
							>
								<CheckCircle className="mr-2 h-4 w-4" />
								Ack
							</Button>
						</div>
					))}
					{alerts.data?.items.length === 0 && (
						<div className="rounded-md border p-4 text-sm text-muted-foreground">
							No usage alerts.
						</div>
					)}
				</CardContent>
			</Card>
		</div>
	);
}

function UsageOverviewSection({
	profile,
	hasAdminAccess,
}: {
	profile: IProfile | undefined;
	hasAdminAccess: boolean;
}) {
	const backend = useBackend();
	const [period, setPeriod] = useState<IUsageLimitPeriod>("monthly");
	const overview = useQuery<IAdminUsageOverview>({
		queryKey: ["admin", "usage", period],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<IAdminUsageOverview>(
				profile,
				`admin/usage/overview?period=${period}`,
			);
		},
		enabled: Boolean(profile && hasAdminAccess),
	});

	if (!hasAdminAccess) return null;

	return (
		<div className="space-y-4">
			<div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
				<div>
					<h2 className="text-lg font-semibold">Usage Dashboard</h2>
					<p className="text-sm text-muted-foreground">
						Remote model spend, app executions, user activity, and limits.
					</p>
				</div>
				<Select
					value={period}
					onValueChange={(value) => setPeriod(value as IUsageLimitPeriod)}
				>
					<SelectTrigger className="w-full sm:w-40">
						<SelectValue />
					</SelectTrigger>
					<SelectContent>
						{PERIODS.map((item) => (
							<SelectItem key={item} value={item}>
								{item[0].toUpperCase() + item.slice(1)}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			</div>

			<div className="grid gap-4 xl:grid-cols-[2fr_1fr]">
				<ActivityTrendChart
					overview={overview.data}
					loading={overview.isLoading}
				/>
				<UsageHealthSummary
					overview={overview.data}
					loading={overview.isLoading}
					period={period}
				/>
			</div>

			<div className="grid gap-4 xl:grid-cols-[1.5fr_1fr]">
				<SpendTokenTrendChart
					overview={overview.data}
					loading={overview.isLoading}
				/>
				<SpendMixChart overview={overview.data} loading={overview.isLoading} />
			</div>

			<div className="grid gap-4 lg:grid-cols-2">
				<TopAppsActivityChart
					overview={overview.data}
					loading={overview.isLoading}
				/>
				<TopModelsChart overview={overview.data} loading={overview.isLoading} />
			</div>

			<div className="grid gap-4 xl:grid-cols-3">
				<LimitUtilization
					overview={overview.data}
					loading={overview.isLoading}
					period={period}
				/>
				<PowerUsers overview={overview.data} loading={overview.isLoading} />
				<TechnicalUsers
					overview={overview.data}
					loading={overview.isLoading}
					profile={profile}
					period={period}
				/>
			</div>

			{profile && <UsageOperations profile={profile} period={period} />}

			<Card>
				<CardHeader>
					<CardTitle className="text-base">Top Apps</CardTitle>
					<CardDescription>
						Set rolling {period} cost and token limits per app.
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-4">
					<div className="space-y-2">
						{overview.isLoading && <Skeleton className="h-32 w-full" />}
						{overview.data?.apps.map((app) => (
							<div
								key={app.appId ?? "unknown"}
								className="grid gap-3 rounded-md border p-3 lg:grid-cols-[1.2fr_0.8fr_1.2fr]"
							>
								<div className="min-w-0">
									<div className="truncate text-sm font-medium">
										{app.appName ?? app.appId ?? "Unknown app"}
									</div>
									<div className="truncate text-xs text-muted-foreground">
										{app.appId ?? "usage without app context"}
									</div>
								</div>
								<div className="grid grid-cols-3 gap-2 text-xs">
									<div>
										<div className="text-muted-foreground">Cost</div>
										<div className="font-medium">
											{formatCost(app.totalPrice)}
										</div>
									</div>
									<div>
										<div className="text-muted-foreground">Tokens</div>
										<div className="font-medium">
											{formatCount(app.totalTokens)}
										</div>
									</div>
									<div>
										<div className="text-muted-foreground">Runs</div>
										<div className="font-medium">
											{formatCount(app.executions)}
										</div>
									</div>
								</div>
								{profile && (
									<LimitEditor app={app} period={period} profile={profile} />
								)}
							</div>
						))}
						{overview.data && overview.data.apps.length === 0 && (
							<div className="rounded-md border p-4 text-sm text-muted-foreground">
								No usage recorded for this period.
							</div>
						)}
					</div>
					{profile && <ManualLimitEditor profile={profile} period={period} />}
				</CardContent>
			</Card>

			<div className="grid gap-4 lg:grid-cols-2">
				<Card>
					<CardHeader>
						<CardTitle className="text-base">Top Users</CardTitle>
					</CardHeader>
					<CardContent className="space-y-2">
						{overview.isLoading && <Skeleton className="h-28 w-full" />}
						{overview.data?.users.map((user) => (
							<div
								key={user.userId ?? "unknown"}
								className="grid grid-cols-[1fr_auto] gap-3 rounded-md border p-3 text-sm"
							>
								<div className="min-w-0">
									<div className="truncate font-medium">
										{user.displayName ??
											user.email ??
											user.userId ??
											"Unknown user"}
									</div>
									<div className="truncate text-xs text-muted-foreground">
										{user.email ?? user.userId ?? "usage without user context"}
									</div>
								</div>
								<div className="text-right">
									<div className="font-medium">
										{formatCost(user.totalPrice)}
									</div>
									<div className="text-xs text-muted-foreground">
										{formatCount(user.totalTokens)} tokens
									</div>
								</div>
							</div>
						))}
					</CardContent>
				</Card>
				<Card>
					<CardHeader>
						<CardTitle className="text-base">Top Remote Models</CardTitle>
					</CardHeader>
					<CardContent className="space-y-2">
						{overview.isLoading && <Skeleton className="h-28 w-full" />}
						{overview.data?.models.map((model) => (
							<div
								key={`${model.kind}:${model.provider ?? ""}:${model.modelId}`}
								className="grid grid-cols-[1fr_auto] gap-3 rounded-md border p-3 text-sm"
							>
								<div className="min-w-0">
									<div className="truncate font-medium">{model.modelId}</div>
									<div className="truncate text-xs text-muted-foreground">
										{model.kind}
										{model.provider ? ` - ${model.provider}` : ""}
									</div>
								</div>
								<div className="text-right">
									<div className="font-medium">{formatCost(model.price)}</div>
									<div className="text-xs text-muted-foreground">
										{formatCount(model.invocations)} calls
									</div>
								</div>
							</div>
						))}
					</CardContent>
				</Card>
			</div>
		</div>
	);
}

interface GovernanceScoresSummaryData {
	criticalApps: number;
	flaggedApps: number;
	totalApps: number;
	worstApps: Array<{
		appId: string;
		appName: string | null;
		worstScore: number;
		security: number;
		privacy: number;
	}>;
}

interface AiActExportRow {
	appId: string;
	appName: string | null;
	riskCategory: string;
	status: string;
	conformityScore: number | null;
	conformityBand: string | null;
	updatedAt: string;
}

function AiActConformityPreview({
	profile,
}: { profile: IProfile | undefined }) {
	const backend = useBackend();
	const features = useFeatures();
	const aiActEnabled = features.data?.ai_act === true;

	const inventory = useQuery<AiActExportRow[]>({
		queryKey: ["admin", "ai-act", "dashboard", "export"],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<AiActExportRow[]>(
				profile,
				"admin/ai-act/inventory/export?format=json",
			);
		},
		enabled: !!profile && aiActEnabled,
	});

	const stats = useMemo(() => {
		const rows = inventory.data ?? [];
		const assessed = rows.filter(
			(r) => r.status.toUpperCase() !== "UNASSESSED",
		);
		const prohibited = rows.filter(
			(r) => r.riskCategory === "PROHIBITED",
		).length;
		const high = rows.filter((r) => r.riskCategory === "HIGH").length;
		const limited = rows.filter((r) => r.riskCategory === "LIMITED").length;
		const scored = rows.filter((r) => typeof r.conformityScore === "number");
		const avgConformity =
			scored.length > 0
				? Math.round(
						scored.reduce((sum, r) => sum + (r.conformityScore ?? 0), 0) /
							scored.length,
					)
				: null;
		return {
			total: assessed.length,
			prohibited,
			high,
			limited,
			avgConformity,
		};
	}, [inventory.data]);

	if (!aiActEnabled) return null;

	return (
		<div className="space-y-2 rounded-lg border border-indigo-500/30 bg-indigo-500/5 p-3">
			<div className="flex flex-col items-start gap-2 min-[400px]:flex-row min-[400px]:items-center min-[400px]:justify-between">
				<div className="flex items-center gap-2 text-sm font-medium">
					<Scale className="h-4 w-4 text-indigo-500" />
					EU AI Act Conformity
				</div>
				<Button asChild variant="ghost" size="sm" className="h-7 px-2 text-xs">
					<Link href="/admin/ai-act">Open Inventory</Link>
				</Button>
			</div>
			{inventory.isLoading ? (
				<Skeleton className="h-16 w-full" />
			) : (
				<div className="grid grid-cols-2 gap-2 sm:grid-cols-5">
					<div className="rounded-md border bg-background p-2">
						<div className="text-[11px] text-muted-foreground">Assessed</div>
						<div className="text-lg font-semibold">{stats.total}</div>
					</div>
					<div
						className={`rounded-md border bg-background p-2 ${
							stats.prohibited > 0 ? "border-red-500/50" : ""
						}`}
					>
						<div className="text-[11px] text-muted-foreground">Prohibited</div>
						<div
							className={`text-lg font-semibold ${
								stats.prohibited > 0 ? "text-red-600 dark:text-red-400" : ""
							}`}
						>
							{stats.prohibited}
						</div>
					</div>
					<div
						className={`rounded-md border bg-background p-2 ${
							stats.high > 0 ? "border-amber-500/50" : ""
						}`}
					>
						<div className="text-[11px] text-muted-foreground">High-risk</div>
						<div
							className={`text-lg font-semibold ${
								stats.high > 0 ? "text-amber-600 dark:text-amber-400" : ""
							}`}
						>
							{stats.high}
						</div>
					</div>
					<div className="rounded-md border bg-background p-2">
						<div className="text-[11px] text-muted-foreground">Limited</div>
						<div className="text-lg font-semibold">{stats.limited}</div>
					</div>
					<div className="rounded-md border bg-background p-2">
						<div className="text-[11px] text-muted-foreground">
							Avg. conformity
						</div>
						<div className="text-lg font-semibold">
							{stats.avgConformity ?? "—"}
							{stats.avgConformity !== null && (
								<span className="text-xs text-muted-foreground">%</span>
							)}
						</div>
					</div>
				</div>
			)}
		</div>
	);
}

function GovernanceScoresSummary({
	profile,
}: {
	profile: IProfile | undefined;
}) {
	const backend = useBackend();

	const summary = useQuery<GovernanceScoresSummaryData>({
		queryKey: ["admin", "governance", "scores", "summary"],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<GovernanceScoresSummaryData>(
				profile,
				"admin/governance/scores/summary",
			);
		},
		enabled: !!profile,
	});

	const hasCriticalIssues = (summary.data?.criticalApps ?? 0) > 0;
	const hasFlaggedIssues = (summary.data?.flaggedApps ?? 0) > 0;

	return (
		<Card
			className={
				hasCriticalIssues
					? "border-red-500/50 bg-red-500/5"
					: hasFlaggedIssues
						? "border-yellow-500/50 bg-yellow-500/5"
						: ""
			}
		>
			<CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
				<div>
					<CardTitle className="flex items-center gap-2 text-base">
						<Shield
							className={`h-4 w-4 ${
								hasCriticalIssues
									? "text-red-500"
									: hasFlaggedIssues
										? "text-yellow-500"
										: "text-green-500"
							}`}
						/>
						AI Inventory & Governance
					</CardTitle>
					<CardDescription>
						EU AI Act conformity together with security and quality scores
						across published apps
					</CardDescription>
				</div>
				<Button asChild variant="outline" size="sm">
					<Link href="/admin/ai-act">View Full Inventory</Link>
				</Button>
			</CardHeader>
			<CardContent>
				{summary.isLoading ? (
					<Skeleton className="h-32 w-full" />
				) : summary.error ? (
					<div className="rounded-md border border-destructive/40 p-4 text-center text-sm text-destructive">
						Failed to load governance scores. Please check the API logs.
					</div>
				) : summary.data ? (
					<div className="space-y-4">
						<AiActConformityPreview profile={profile} />

						<div className="grid gap-3 sm:grid-cols-3">
							<div className="rounded-lg border p-3">
								<div className="text-xs text-muted-foreground">Total Apps</div>
								<div className="text-2xl font-semibold">
									{summary.data.totalApps ?? 0}
								</div>
							</div>
							<div
								className={`rounded-lg border p-3 ${
									(summary.data.criticalApps ?? 0) > 0
										? "border-red-500/50 bg-red-500/5"
										: ""
								}`}
							>
								<div className="text-xs text-muted-foreground">
									Critical Issues
								</div>
								<div
									className={`text-2xl font-semibold ${
										(summary.data.criticalApps ?? 0) > 0
											? "text-red-600 dark:text-red-400"
											: ""
									}`}
								>
									{summary.data.criticalApps ?? 0}
								</div>
								<div className="text-xs text-muted-foreground">Score ≤ 3</div>
							</div>
							<div
								className={`rounded-lg border p-3 ${
									(summary.data.flaggedApps ?? 0) > 0
										? "border-yellow-500/50 bg-yellow-500/5"
										: ""
								}`}
							>
								<div className="text-xs text-muted-foreground">
									Flagged Apps
								</div>
								<div
									className={`text-2xl font-semibold ${
										(summary.data.flaggedApps ?? 0) > 0
											? "text-yellow-600 dark:text-yellow-400"
											: ""
									}`}
								>
									{summary.data.flaggedApps ?? 0}
								</div>
								<div className="text-xs text-muted-foreground">Score ≤ 6</div>
							</div>
						</div>

						{summary.data.worstApps && summary.data.worstApps.length > 0 && (
							<div className="space-y-2">
								<div className="text-sm font-medium">
									Apps Requiring Attention
								</div>
								{summary.data.worstApps.map((app) => (
									<Link
										key={app.appId}
										href={`/admin/ai-act?id=${encodeURIComponent(app.appId)}`}
										className="block"
									>
										<div className="grid grid-cols-[1fr_auto] gap-3 rounded-md border p-3 text-sm transition-colors hover:border-primary/40">
											<div className="min-w-0">
												<div className="truncate font-medium">
													{app.appName ?? app.appId}
												</div>
												<div className="flex items-center gap-3 text-xs text-muted-foreground">
													<span className="truncate">{app.appId}</span>
												</div>
											</div>
											<div className="flex items-center gap-3">
												<div className="text-right text-xs">
													<div className="text-muted-foreground">Security</div>
													<div
														className={`font-semibold ${
															app.security >= 7
																? "text-green-600 dark:text-green-400"
																: app.security >= 4
																	? "text-yellow-600 dark:text-yellow-400"
																	: "text-red-600 dark:text-red-400"
														}`}
													>
														{app.security}
													</div>
												</div>
												<div className="text-right text-xs">
													<div className="text-muted-foreground">Privacy</div>
													<div
														className={`font-semibold ${
															app.privacy >= 7
																? "text-green-600 dark:text-green-400"
																: app.privacy >= 4
																	? "text-yellow-600 dark:text-yellow-400"
																	: "text-red-600 dark:text-red-400"
														}`}
													>
														{app.privacy}
													</div>
												</div>
												<div className="text-right">
													<div className="text-xs text-muted-foreground">
														Worst
													</div>
													<div
														className={`text-lg font-bold ${
															app.worstScore >= 7
																? "text-green-600 dark:text-green-400"
																: app.worstScore >= 4
																	? "text-yellow-600 dark:text-yellow-400"
																	: "text-red-600 dark:text-red-400"
														}`}
													>
														{app.worstScore}
													</div>
												</div>
											</div>
										</div>
									</Link>
								))}
							</div>
						)}
					</div>
				) : (
					<div className="rounded-md border p-4 text-center text-sm text-muted-foreground">
						No governance data available yet. Scores will appear after apps are
						published and analyzed.
					</div>
				)}
			</CardContent>
		</Card>
	);
}

export interface AdminDashboardPageProps {
	infoEnabled?: boolean;
	infoDependencyKey?: unknown[];
}

export function AdminDashboardPage({
	infoEnabled = true,
	infoDependencyKey = [],
}: AdminDashboardPageProps) {
	const backend = useBackend();
	const queryClient = useQueryClient();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);
	const info = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		infoEnabled,
		infoDependencyKey,
	);

	const perms = useMemo(
		() => new GlobalPermission(info.data?.permission ?? 0),
		[info.data?.permission],
	);
	const features = useFeatures().data;
	const visibleSections = useMemo(
		() =>
			ADMIN_SECTIONS.filter(
				(section) =>
					!section.feature ||
					Boolean(
						(features as Record<string, boolean> | undefined)?.[
							section.feature
						],
					),
			),
		[features],
	);
	const hasAdminAccess = perms.hasPermission(GlobalPermission.Admin);
	const hasPackageAccess = perms.hasPermission(GlobalPermission.ManagePackages);

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

	const ensureWasmArtifacts = useMutation({
		mutationFn: async () => {
			if (!profile.data) throw new Error("Profile not loaded");
			return backend.apiState.post<AdminEnsureWasmArtifactsResponse>(
				profile.data,
				"admin/packages/ensure-wasm-artifacts",
				{},
			);
		},
		onSuccess: async () => {
			await queryClient.invalidateQueries({
				queryKey: ["admin", "packages"],
			});
		},
	});

	const statsLoading = packageStats.isLoading;
	const ensureResult = ensureWasmArtifacts.data;
	const ensureError =
		ensureWasmArtifacts.error instanceof Error
			? ensureWasmArtifacts.error.message
			: null;

	return (
		<main className="flex h-full min-h-0 min-w-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="min-w-0 flex-1 overflow-y-auto overflow-x-hidden p-3 sm:p-6">
				<div className="mx-auto min-w-0 max-w-7xl space-y-4 sm:space-y-6">
					<div>
						<h1 className="text-2xl font-bold sm:text-3xl">Admin Dashboard</h1>
						<p className="text-muted-foreground">
							Central hub for registry, publishing, usage, and learning content.
						</p>
					</div>

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
									Packages are waiting for approval before they can be
									published.
								</CardDescription>
							</CardHeader>
							<CardContent>
								<Button asChild variant="outline" size="sm">
									<Link href="/admin/packages">Review Now</Link>
								</Button>
							</CardContent>
						</Card>
					)}

					{hasPackageAccess && (
						<Card>
							<CardHeader className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
								<div>
									<CardTitle className="flex items-center gap-2 text-base">
										<Cpu className="h-4 w-4 text-green-500" />
										WASM Artifact Compatibility
									</CardTitle>
									<CardDescription>
										Check active package versions for current Linux Wasmtime
										artifacts and queue missing compiles.
									</CardDescription>
								</div>
								<Button
									size="sm"
									variant="outline"
									disabled={!profile.data || ensureWasmArtifacts.isPending}
									onClick={() => ensureWasmArtifacts.mutate()}
								>
									<RefreshCw
										className={`mr-2 h-4 w-4 ${ensureWasmArtifacts.isPending ? "animate-spin" : ""}`}
									/>
									Check Artifacts
								</Button>
							</CardHeader>
							{(ensureResult || ensureError) && (
								<CardContent>
									{ensureResult ? (
										<div className="grid gap-3 text-sm sm:grid-cols-4">
											<div>
												<div className="text-muted-foreground">Target</div>
												<div className="font-medium">
													{ensureResult.targetPlatform}
												</div>
											</div>
											<div>
												<div className="text-muted-foreground">Checked</div>
												<div className="font-medium">
													{ensureResult.checkedVersions}
												</div>
											</div>
											<div>
												<div className="text-muted-foreground">
													Jobs started
												</div>
												<div className="font-medium">
													{ensureResult.jobsStarted}
												</div>
											</div>
											<div>
												<div className="text-muted-foreground">Ready</div>
												<div className="font-medium">
													{ensureResult.alreadyAvailable}
												</div>
											</div>
											{ensureResult.failed > 0 && (
												<div className="sm:col-span-4 rounded-md border border-destructive/40 p-3 text-destructive">
													Failed dispatches: {ensureResult.failed}
												</div>
											)}
										</div>
									) : (
										<div className="rounded-md border border-destructive/40 p-3 text-sm text-destructive">
											{ensureError}
										</div>
									)}
								</CardContent>
							)}
						</Card>
					)}

					<UsageOverviewSection
						profile={profile.data}
						hasAdminAccess={hasAdminAccess}
					/>

					{hasAdminAccess && <GovernanceScoresSummary profile={profile.data} />}

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
							value={
								openSolutions.isLoading
									? "\u2014"
									: (openSolutions.data?.total ?? 0)
							}
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
							value={
								profiles.isLoading ? "\u2014" : (profiles.data?.length ?? 0)
							}
							description="Reusable user profiles"
							icon={<Users className="h-4 w-4 text-purple-500" />}
							loading={profiles.isLoading}
							href="/admin/user/edit"
						/>
					</div>

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

					{hasAdminAccess && features?.telemetry === true && (
						<div className="grid gap-4">
							<DashboardTelemetryWidget profile={profile.data} />
							<DashboardTelemetryIssuesWidget profile={profile.data} />
							<DashboardTelemetryTracesWidget profile={profile.data} />
							<DashboardTelemetryAlertsWidget profile={profile.data} />
						</div>
					)}

					{perms.hasPermission(GlobalPermission.ReadLogs) && (
						<div className="grid gap-4 lg:grid-cols-2">
							<DashboardErrorWidget profile={profile.data} />
							<DashboardChainWidget profile={profile.data} />
						</div>
					)}

					<div>
						<h2 className="mb-3 text-lg font-semibold">Manage</h2>
						<div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
							{visibleSections.map((section) => (
								<SectionCard
									key={section.title}
									section={section}
									hasAccess={
										perms.hasPermission(section.permission) ||
										Boolean(
											section.alternatePermissions?.some((permission) =>
												perms.hasPermission(permission),
											),
										)
									}
								/>
							))}
						</div>
					</div>
				</div>
			</div>
		</main>
	);
}
