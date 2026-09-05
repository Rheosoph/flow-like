"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type UseQueryResult,
	useMutation,
	useQuery,
	useQueryClient,
} from "@tanstack/react-query";
import {
	Activity,
	ArrowRight,
	BellRing,
	BookOpen,
	Check,
	CheckCircle2,
	ChevronRight,
	CircleDot,
	Clock3,
	Cpu,
	LayoutGrid,
	Lightbulb,
	type LucideIcon,
	Package,
	RefreshCw,
	ShieldAlert,
	UserCog,
	Users,
	Wrench,
} from "lucide-react";
import Link from "next/link";
import { Suspense, lazy, useMemo, useState } from "react";
import { useFeatures } from "../../../hooks/use-features";
import { useInvoke } from "../../../hooks/use-invoke";
import { GlobalPermission } from "../../../lib/permission/global-permission";
import type { IProfile } from "../../../lib/schema/profile/profile";
import type { AdminEnsureWasmArtifactsResponse } from "../../../lib/schema/wasm";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Skeleton } from "../../ui/skeleton";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../ui/tabs";
import {
	normalizeRegistryStats,
	prioritizeDashboardQueues,
	readDashboardCount,
} from "./admin-dashboard-model";
import {
	ADMIN_SECTIONS,
	AdminDashboardNavigation,
} from "./admin-dashboard-navigation";

const UsageOverviewSection = lazy(() =>
	import("./admin-dashboard-details").then((module) => ({
		default: module.UsageOverviewSection,
	})),
);
const GovernanceScoresSummary = lazy(() =>
	import("./admin-dashboard-details").then((module) => ({
		default: module.GovernanceScoresSummary,
	})),
);
const SystemHealth = lazy(() => import("./admin-dashboard-system"));

interface GovernanceSummary {
	criticalApps: number;
	flaggedApps: number;
	totalApps: number;
}

function useDashboardQuery<T>({
	profile,
	permission,
	accountId,
	queryKey,
	enabled,
	path,
	select,
}: {
	profile: IProfile | undefined;
	permission: number | undefined;
	accountId: string | undefined;
	queryKey: string[];
	enabled: boolean;
	path: string;
	select: (data: unknown) => T;
}) {
	const backend = useBackend();
	return useQuery({
		queryKey: [...queryKey, profile?.hub, profile?.id, permission, accountId],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			return backend.apiState.get<unknown>(profile, path);
		},
		select,
		enabled: Boolean(profile && enabled),
		staleTime: 60_000,
		retry: 1,
		meta: { persist: false, adminDashboard: true },
	});
}
function responseCount(data: unknown, field = "total") {
	return readDashboardCount(
		(data as Record<string, unknown> | null)?.[field],
		field,
	);
}
interface Queue {
	id: string;
	title: string;
	description: string;
	href: string;
	icon: LucideIcon;
	priority: number;
	count: number | null;
	loading: boolean;
	error: boolean;
	retry: () => unknown;
	critical?: boolean;
}
function queryState<T>(query: UseQueryResult<T>, count: number | undefined) {
	return {
		count: query.isError || count === undefined ? null : count,
		loading: query.isPending,
		error: query.isError,
		retry: () => query.refetch(),
	};
}
function QueueRow({ queue }: { queue: Queue }) {
	const { t } = useTranslation("admin");
	const Icon = queue.icon;
	const actionable = queue.count !== null && queue.count > 0;
	return (
		<div className="group flex items-center gap-2 border-t border-border/60 px-4 first:border-0 sm:px-5">
			<Link
				prefetch={false}
				href={queue.href}
				className="flex min-w-0 flex-1 items-center gap-3 rounded-lg py-4 outline-none focus-visible:ring-2 focus-visible:ring-ring sm:gap-4"
			>
				<span
					className={cn(
						"flex size-10 shrink-0 items-center justify-center rounded-xl",
						queue.critical && actionable
							? "bg-destructive/10 text-destructive"
							: actionable
								? "bg-primary/10 text-primary"
								: "bg-muted/60 text-muted-foreground",
					)}
				>
					<Icon aria-hidden="true" className="size-4.5" />
				</span>
				<span className="min-w-0 flex-1">
					<span className="block text-sm font-medium">{queue.title}</span>
					<span className="mt-1 block text-xs leading-relaxed text-muted-foreground">
						{queue.description}
					</span>
				</span>
				{queue.loading ? (
					<Skeleton className="h-7 w-10" />
				) : queue.error ? (
					<Badge
						variant="outline"
						className="shrink-0 border-destructive/30 text-destructive"
					>
						{t("dashboardUnavailable", "Unavailable")}
					</Badge>
				) : (
					<span
						className={cn(
							"flex min-w-8 shrink-0 items-center justify-center rounded-lg px-2 py-1 text-sm font-semibold tabular-nums",
							actionable
								? queue.critical
									? "bg-destructive/10 text-destructive"
									: "bg-primary/10 text-primary"
								: "text-muted-foreground",
						)}
					>
						{actionable ? (
							queue.count?.toLocaleString()
						) : (
							<>
								<Check aria-hidden="true" className="size-4" />
								<span className="sr-only">
									{t("dashboardNoPendingItems", "No pending items")}
								</span>
							</>
						)}
					</span>
				)}
				<ChevronRight
					aria-hidden="true"
					className="hidden size-4 shrink-0 text-muted-foreground/50 group-hover:text-foreground sm:block"
				/>
			</Link>
			{queue.error && (
				<Button
					variant="ghost"
					size="icon"
					className="size-8 shrink-0"
					onClick={queue.retry}
					aria-label={`${t("dashboardRetry", "Retry")}: ${queue.title}`}
				>
					<RefreshCw className="size-3.5" />
				</Button>
			)}
		</div>
	);
}
function Metric({
	label,
	value,
	loading,
	error,
	href,
}: {
	label: string;
	value: number | undefined;
	loading: boolean;
	error: boolean;
	href: string;
}) {
	const { t } = useTranslation("admin");
	return (
		<Link
			prefetch={false}
			href={href}
			className="group min-w-0 rounded-xl border bg-card px-4 py-4 shadow-xs outline-none transition-colors hover:border-primary/40 focus-visible:ring-2 focus-visible:ring-ring sm:px-5"
		>
			<div className="flex items-center justify-between gap-2 text-xs text-muted-foreground">
				<span>{label}</span>
				<ArrowRight
					aria-hidden="true"
					className="size-3.5 text-muted-foreground/50 group-hover:text-primary"
				/>
			</div>
			{loading ? (
				<Skeleton className="mt-3 h-8 w-16" />
			) : (
				<div
					className={cn(
						"mt-2 font-semibold tracking-tight tabular-nums",
						error || value === undefined
							? "text-sm text-muted-foreground"
							: "text-2xl",
					)}
				>
					{error || value === undefined
						? t("dashboardUnavailable", "Unavailable")
						: value.toLocaleString()}
				</div>
			)}
		</Link>
	);
}
function PanelLoading() {
	const { t } = useTranslation("admin");
	return (
		<div aria-live="polite" className="space-y-4">
			<span className="sr-only">
				{t("dashboardLoadingView", "Loading view")}
			</span>
			<Skeleton className="h-12 w-64" />
			<Skeleton className="h-72 w-full rounded-xl" />
		</div>
	);
}
function Maintenance({ profile }: { profile: IProfile }) {
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const ensure = useMutation({
		mutationFn: () =>
			backend.apiState.post<AdminEnsureWasmArtifactsResponse>(
				profile,
				"admin/packages/ensure-wasm-artifacts",
				{},
			),
		onSuccess: () =>
			queryClient.invalidateQueries({ queryKey: ["admin", "packages"] }),
	});
	return (
		<section className="rounded-xl border bg-card p-5 sm:p-6">
			<div className="flex flex-col justify-between gap-4 sm:flex-row sm:items-start">
				<div className="max-w-2xl">
					<h2 className="flex items-center gap-2 font-semibold">
						<Cpu aria-hidden="true" className="size-4 text-muted-foreground" />
						{t("wasmArtifactCompatibility", "WASM artifact compatibility")}
					</h2>
					<p className="mt-2 text-sm leading-relaxed text-muted-foreground">
						{t(
							"dashboardArtifactDescription",
							"Check active package versions for the current Linux runtime. Missing compiled artifacts are queued for a rebuild.",
						)}
					</p>
				</div>
				<Button
					variant="outline"
					disabled={ensure.isPending}
					onClick={() => ensure.mutate()}
				>
					<RefreshCw
						className={cn("size-4", ensure.isPending && "animate-spin")}
					/>
					{t("dashboardCheckAndQueue", "Check & queue builds")}
				</Button>
			</div>
			{ensure.isError && (
				<div
					role="alert"
					className="mt-5 rounded-lg border border-destructive/30 p-3 text-sm text-destructive"
				>
					{ensure.error.message}
				</div>
			)}
			{ensure.data && (
				<div aria-live="polite" className="mt-5 space-y-3">
					<p className="text-xs text-muted-foreground">
						{ensure.data.targetPlatform}
					</p>
					<dl className="grid grid-cols-2 gap-3 sm:grid-cols-4">
						{[
							[t("checked", "Checked"), ensure.data.checkedVersions],
							[t("jobsStarted", "Jobs started"), ensure.data.jobsStarted],
							[t("ready", "Ready"), ensure.data.alreadyAvailable],
							[t("dashboardFailed", "Failed"), ensure.data.failed],
						].map(([label, count]) => (
							<div key={label} className="rounded-lg bg-muted/40 p-3">
								<dt className="text-xs text-muted-foreground">{label}</dt>
								<dd className="mt-1 text-xl font-semibold tabular-nums">
									{count}
								</dd>
							</div>
						))}
					</dl>
					{ensure.data.failed > 0 && (
						<p className="text-sm text-destructive">
							{t(
								"dashboardBuildFailures",
								"Some builds could not be queued. Check the package registry and try again.",
							)}
						</p>
					)}
				</div>
			)}
		</section>
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
	const { t } = useTranslation("admin");
	const backend = useBackend();
	const queryClient = useQueryClient();
	const [view, setView] = useState("overview");
	const [refreshing, setRefreshing] = useState(false);
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		infoEnabled,
		infoDependencyKey,
	);
	const info = useInvoke(
		backend.userState.getInfo,
		backend.userState,
		[],
		infoEnabled,
		infoDependencyKey,
	);
	const featureQuery = useFeatures();
	const perms = useMemo(
		() => new GlobalPermission(infoEnabled ? (info.data?.permission ?? 0) : 0),
		[infoEnabled, info.data?.permission],
	);
	const ready = infoEnabled && info.isSuccess && profile.isSuccess;
	const admin = ready && perms.hasPermission(GlobalPermission.Admin);
	const publishing =
		ready && perms.hasPermission(GlobalPermission.ReadPublishing);
	const packages =
		ready && perms.hasPermission(GlobalPermission.ManagePackages);
	const solutions =
		ready && perms.hasPermission(GlobalPermission.ReadSolutions);
	const logs = ready && perms.hasPermission(GlobalPermission.ReadLogs);
	const telemetry = admin && featureQuery.data?.telemetry === true;
	const context = {
		profile: profile.data,
		permission: info.data?.permission,
		accountId: info.data?.id,
	};
	const packageStats = useDashboardQuery({
		...context,
		queryKey: ["admin", "packages", "stats"],
		enabled: packages,
		path: "admin/packages/stats",
		select: normalizeRegistryStats,
	});
	const appReviews = useDashboardQuery({
		...context,
		queryKey: ["admin", "publication", "requests", "dashboard"],
		enabled: publishing,
		path: "admin/publication/requests?status=PENDING&page=1&limit=1",
		select: (data) => responseCount(data),
	});
	const suiteReviews = useDashboardQuery({
		...context,
		queryKey: ["admin", "publication", "suites", "dashboard"],
		enabled: publishing,
		path: "admin/publication/suites?status=PENDING&page=1&limit=1",
		select: (data) => responseCount(data),
	});
	const solutionReviews = useDashboardQuery({
		...context,
		queryKey: ["admin", "solutions", "open-count"],
		enabled: solutions,
		path: "admin/solutions?page=1&limit=1&status=PENDING_REVIEW",
		select: (data) => responseCount(data),
	});
	const governance = useDashboardQuery({
		...context,
		queryKey: ["admin", "governance", "scores", "summary"],
		enabled: publishing,
		path: "admin/governance/scores/summary",
		select: (data) => {
			const raw = data as GovernanceSummary;
			return {
				...raw,
				criticalApps: responseCount(raw, "criticalApps"),
				flaggedApps: responseCount(raw, "flaggedApps"),
				totalApps: responseCount(raw, "totalApps"),
			};
		},
	});
	const alerts = useDashboardQuery({
		...context,
		queryKey: ["admin", "telemetry", "alerts", "dashboard-count"],
		enabled: telemetry,
		path: "admin/telemetry/alerts/events?hours=168&status=triggered&page=0&page_size=1",
		select: (data) => responseCount(data, "unacknowledged"),
	});
	const queues: Queue[] = [];
	if (publishing) {
		queues.push({
			id: "governance",
			title: t("dashboardGovernanceFindings", "Governance findings"),
			description: governance.data?.criticalApps
				? t(
						"dashboardCriticalFindings",
						governance.data.criticalApps === 1
							? "{{count}} app has critical scores. Review it first."
							: "{{count}} apps have critical scores. Review these first.",
						{ count: governance.data.criticalApps },
					)
				: t(
						"dashboardFlaggedFindings",
						"Apps with security or quality scores that need review.",
					),
			href: "/admin/governance/scores",
			icon: ShieldAlert,
			priority: governance.data?.criticalApps ? 0 : 2,
			critical: (governance.data?.criticalApps ?? 0) > 0,
			...queryState(
				governance,
				governance.data
					? governance.data.criticalApps + governance.data.flaggedApps
					: undefined,
			),
		});
		queues.push({
			id: "apps",
			title: t("dashboardAppPublications", "App publications"),
			description: t(
				"dashboardAppPublicationsDescription",
				"Review apps waiting to be published.",
			),
			href: "/admin/governance",
			icon: BookOpen,
			priority: 3,
			...queryState(appReviews, appReviews.data),
		});
		queues.push({
			id: "suites",
			title: t("dashboardSuitePublications", "Suite publications"),
			description: t(
				"dashboardSuitePublicationsDescription",
				"Review suites waiting to be published.",
			),
			href: "/admin/governance/suites",
			icon: LayoutGrid,
			priority: 4,
			...queryState(suiteReviews, suiteReviews.data),
		});
	}
	if (telemetry)
		queues.push({
			id: "alerts",
			title: t("dashboardActiveAlerts", "Unacknowledged alerts"),
			description: t(
				"dashboardActiveAlertsDescription",
				"Triggered telemetry alerts from the last 7 days.",
			),
			href: "/admin/telemetry/alerts",
			icon: BellRing,
			priority: 1,
			critical: true,
			...queryState(alerts, alerts.data),
		});
	if (packages)
		queues.push({
			id: "packages",
			title: t("dashboardPackageReviews", "Package reviews"),
			description: t(
				"dashboardPackageReviewsDescription",
				"Approve packages before they reach the registry.",
			),
			href: "/admin/packages",
			icon: Package,
			priority: 5,
			...queryState(packageStats, packageStats.data?.pendingReview),
		});
	if (solutions)
		queues.push({
			id: "solutions",
			title: t("dashboardSolutionReviews", "Solution requests"),
			description: t(
				"dashboardSolutionReviewsDescription",
				"New requests waiting for an initial review.",
			),
			href: "/admin/solutions",
			icon: Lightbulb,
			priority: 6,
			...queryState(solutionReviews, solutionReviews.data),
		});
	const ordered = prioritizeDashboardQueues(queues);
	const attentionCount = queues.filter(
		(queue) => (queue.count ?? 0) > 0,
	).length;
	const pending = queues.some((queue) => queue.loading);
	const failed = queues.some((queue) => queue.error);
	const sections = ready
		? ADMIN_SECTIONS.filter(
				(section) =>
					(!section.feature ||
						(featureQuery.data as Record<string, boolean> | undefined)?.[
							section.feature
						]) &&
					(perms.hasPermission(section.permission) ||
						section.alternatePermissions?.some((permission) =>
							perms.hasPermission(permission),
						)),
			)
		: [];
	const views = [
		"overview",
		...(admin ? ["usage"] : []),
		...(publishing ? ["governance"] : []),
		...(admin || logs ? ["system"] : []),
		...(packages ? ["maintenance"] : []),
	];
	const activeView = views.includes(view) ? view : "overview";
	async function refresh() {
		setRefreshing(true);
		try {
			await queryClient.refetchQueries({
				type: "active",
				predicate: (query) => query.meta?.adminDashboard === true,
			});
		} finally {
			setRefreshing(false);
		}
	}
	const quickLinks = [
		{
			show: perms.hasPermission(GlobalPermission.WriteBits),
			href: "/admin/bits/edit",
			label: t("dashboardManageModels", "Manage bits & models"),
			icon: Cpu,
		},
		{
			show: admin,
			href: "/admin/users",
			label: t("dashboardManageUsers", "Manage users"),
			icon: UserCog,
		},
		{
			show: publishing,
			href: "/admin/governance",
			label: t("dashboardOpenPublications", "Open publications"),
			icon: BookOpen,
		},
		{
			show: perms.hasPermission(GlobalPermission.ReadProfile),
			href: "/admin/profiles",
			label: t("dashboardStarterProfiles", "Starter profiles"),
			icon: Users,
		},
	].filter((item) => item.show);
	const tabs = [
		{
			id: "overview",
			label: t("dashboardOverview", "Overview"),
			icon: LayoutGrid,
		},
		{
			id: "usage",
			label: t("dashboardUsageLimits", "Usage & limits"),
			icon: Activity,
		},
		{
			id: "governance",
			label: t("dashboardGovernance", "Governance"),
			icon: ShieldAlert,
		},
		{
			id: "system",
			label: t("dashboardSystemHealth", "System health"),
			icon: CircleDot,
		},
		{
			id: "maintenance",
			label: t("dashboardMaintenance", "Maintenance"),
			icon: Wrench,
		},
	].filter((tab) => views.includes(tab.id));
	return (
		<main className="flex h-full min-h-0 min-w-0 w-full grow flex-col overflow-hidden bg-background">
			<div className="min-w-0 flex-1 overflow-y-auto overflow-x-hidden">
				<div className="mx-auto w-full max-w-7xl space-y-7 px-4 py-6 sm:px-8 sm:py-8">
					<header className="flex flex-col justify-between gap-4 sm:flex-row sm:items-center">
						<div>
							<div className="mb-2 flex items-center gap-2 text-xs font-medium uppercase tracking-widest text-muted-foreground">
								<span className="size-1.5 rounded-full bg-primary" />
								{t("dashboardAdministration", "Administration")}
							</div>
							<h1 className="text-3xl font-semibold tracking-tight">
								{t("dashboardControlCenter", "Control center")}
							</h1>
							<p className="mt-2 text-sm text-muted-foreground">
								{t(
									"dashboardIntro",
									"Review pending work and manage your platform.",
								)}
							</p>
						</div>
						<Button
							variant="outline"
							size="sm"
							className="w-fit gap-2 bg-card"
							onClick={refresh}
							disabled={refreshing || !ready}
						>
							<RefreshCw
								className={cn("size-3.5", refreshing && "animate-spin")}
							/>
							{refreshing
								? t("dashboardRefreshing", "Refreshing")
								: t("dashboardRefresh", "Refresh data")}
						</Button>
					</header>
					{!infoEnabled ? (
						<div className="rounded-xl border bg-card p-6 text-sm text-muted-foreground">
							{t("dashboardSignIn", "Sign in to access administration.")}
						</div>
					) : profile.isError || info.isError ? (
						<div
							role="alert"
							className="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-destructive/30 bg-card p-5"
						>
							<p className="text-sm">
								{t(
									"dashboardAccessError",
									"We couldn't load your admin access. Try again.",
								)}
							</p>
							<Button
								variant="outline"
								onClick={() => {
									void profile.refetch();
									void info.refetch();
								}}
							>
								{t("dashboardRetry", "Retry")}
							</Button>
						</div>
					) : !ready ? (
						<PanelLoading />
					) : (
						<>
							{featureQuery.isError && (
								<div
									role="alert"
									className="flex flex-wrap items-center justify-between gap-3 rounded-xl border bg-card px-4 py-3 text-sm"
								>
									<span className="text-muted-foreground">
										{t(
											"dashboardFeaturesUnavailable",
											"Some optional tools are unavailable because feature settings could not be loaded.",
										)}
									</span>
									<Button
										variant="ghost"
										size="sm"
										onClick={() => featureQuery.refetch()}
									>
										{t("dashboardRetry", "Retry")}
									</Button>
								</div>
							)}
							<Tabs
								value={activeView}
								onValueChange={setView}
								className="gap-6"
							>
								<TabsList
									aria-label={t("dashboardViews", "Admin views")}
									className="h-auto w-full justify-start gap-1 rounded-none border-b bg-transparent p-0 pb-2"
								>
									{tabs.map((tab) => (
										<TabsTrigger
											key={tab.id}
											value={tab.id}
											className="h-9 flex-none gap-2 px-3 data-[state=active]:bg-muted data-[state=active]:shadow-none"
										>
											<tab.icon aria-hidden="true" className="size-3.5" />
											{tab.label}
										</TabsTrigger>
									))}
								</TabsList>
								<TabsContent value="overview" className="space-y-8">
									<div className="grid items-start gap-5 lg:grid-cols-[minmax(0,1fr)_280px] xl:grid-cols-[minmax(0,1fr)_300px]">
										<section
											aria-labelledby="admin-attention-heading"
											className="overflow-hidden rounded-2xl border bg-card shadow-xs"
										>
											<div className="flex items-center justify-between gap-3 border-b px-4 py-5 sm:px-5">
												<div>
													<h2
														id="admin-attention-heading"
														className="flex items-center gap-2 text-base font-semibold"
													>
														<Clock3
															aria-hidden="true"
															className="size-4 text-primary"
														/>
														{t("dashboardNeedsAttention", "Needs attention")}
													</h2>
													<p className="mt-1 text-xs text-muted-foreground">
														{t(
															"dashboardAttentionDescription",
															"Your review queues, ordered by priority.",
														)}
													</p>
												</div>
												{attentionCount > 0 && (
													<Badge
														variant="secondary"
														className="shrink-0 tabular-nums"
													>
														{t(
															"dashboardAttentionAreas",
															attentionCount === 1
																? "{{count}} queue to review"
																: "{{count}} queues to review",
															{ count: attentionCount },
														)}
													</Badge>
												)}
											</div>
											{!pending &&
												!failed &&
												attentionCount === 0 &&
												queues.length > 0 && (
													<div className="flex items-center gap-3 border-b bg-emerald-500/5 px-5 py-4">
														<CheckCircle2
															aria-hidden="true"
															className="size-5 text-emerald-600 dark:text-emerald-400"
														/>
														<div>
															<p className="text-sm font-medium">
																{t(
																	"dashboardQueuesClear",
																	"You're all caught up",
																)}
															</p>
															<p className="mt-0.5 text-xs text-muted-foreground">
																{t(
																	"dashboardQueuesClearDescription",
																	"No pending items in the queues below.",
																)}
															</p>
														</div>
													</div>
												)}
											{ordered.map((queue) => (
												<QueueRow key={queue.id} queue={queue} />
											))}
											{queues.length === 0 && (
												<div className="p-6 text-sm text-muted-foreground">
													{t(
														"dashboardNoQueues",
														"There are no review queues for your role. Your available tools are below.",
													)}
												</div>
											)}
											{failed && (
												<p
													aria-live="polite"
													className="border-t bg-muted/30 px-5 py-3 text-xs text-muted-foreground"
												>
													{t(
														"dashboardPartialData",
														"Some counts couldn't be loaded. Retry them or open the queue to check.",
													)}
												</p>
											)}
										</section>
										<aside className="space-y-4">
											<div className="rounded-2xl border bg-card p-5 shadow-xs">
												<p className="text-xs font-medium uppercase tracking-wider text-muted-foreground">
													{t("dashboardQuickAccess", "Quick access")}
												</p>
												<div className="mt-3 space-y-1">
													{quickLinks.map((item) => (
														<Link
															prefetch={false}
															key={item.href}
															href={item.href}
															className="flex items-center gap-2.5 rounded-lg py-2.5 text-sm transition-colors hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
														>
															<item.icon
																aria-hidden="true"
																className="size-4 text-muted-foreground"
															/>
															<span className="flex-1">{item.label}</span>
															<ArrowRight
																aria-hidden="true"
																className="size-3.5 text-muted-foreground"
															/>
														</Link>
													))}
												</div>
												{quickLinks.length === 0 && (
													<p className="mt-3 text-sm text-muted-foreground">
														{t(
															"dashboardToolsBelow",
															"Find your available tools below.",
														)}
													</p>
												)}
											</div>
											{(packages || publishing) && (
												<div className="grid grid-cols-2 gap-3 lg:grid-cols-1">
													{packages && (
														<>
															<Metric
																label={t(
																	"dashboardActivePackages",
																	"Active packages",
																)}
																value={packageStats.data?.activePackages}
																loading={packageStats.isPending}
																error={packageStats.isError}
																href="/admin/packages"
															/>
															<Metric
																label={t(
																	"dashboardPackageDownloads",
																	"Package downloads",
																)}
																value={packageStats.data?.totalDownloads}
																loading={packageStats.isPending}
																error={packageStats.isError}
																href="/admin/packages"
															/>
														</>
													)}
													{publishing && (
														<Metric
															label={t(
																"dashboardScoredApps",
																"Apps with governance scores",
															)}
															value={governance.data?.totalApps}
															loading={governance.isPending}
															error={governance.isError}
															href="/admin/governance/scores"
														/>
													)}
												</div>
											)}
										</aside>
									</div>
									<AdminDashboardNavigation sections={sections} />
								</TabsContent>
								{admin && (
									<TabsContent value="usage">
										<Suspense fallback={<PanelLoading />}>
											<UsageOverviewSection
												profile={profile.data}
												hasAdminAccess={admin}
												accountId={info.data?.id}
											/>
										</Suspense>
									</TabsContent>
								)}
								{publishing && (
									<TabsContent value="governance">
										<Suspense fallback={<PanelLoading />}>
											<GovernanceScoresSummary
												profile={profile.data}
												permission={info.data?.permission}
												accountId={info.data?.id}
											/>
										</Suspense>
									</TabsContent>
								)}
								{(admin || logs) && (
									<TabsContent value="system">
										<Suspense fallback={<PanelLoading />}>
											<SystemHealth
												profile={profile.data}
												admin={admin}
												logs={logs}
												telemetry={telemetry}
											/>
										</Suspense>
									</TabsContent>
								)}
								{packages && profile.data && (
									<TabsContent value="maintenance" className="space-y-5">
										<div className="grid gap-3 sm:grid-cols-3">
											<Metric
												label={t(
													"dashboardRegisteredPackages",
													"Registered packages",
												)}
												value={packageStats.data?.totalPackages}
												loading={packageStats.isPending}
												error={packageStats.isError}
												href="/admin/packages"
											/>
											<Metric
												label={t(
													"dashboardRegisteredVersions",
													"Registered versions",
												)}
												value={packageStats.data?.totalVersions}
												loading={packageStats.isPending}
												error={packageStats.isError}
												href="/admin/packages"
											/>
											<Metric
												label={t(
													"dashboardRejectedPackages",
													"Rejected packages",
												)}
												value={packageStats.data?.rejectedPackages}
												loading={packageStats.isPending}
												error={packageStats.isError}
												href="/admin/packages"
											/>
										</div>
										<Maintenance profile={profile.data} />
									</TabsContent>
								)}
							</Tabs>
						</>
					)}
				</div>
			</div>
		</main>
	);
}
