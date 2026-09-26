"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	ActivityIcon,
	ArrowRightIcon,
	CheckCircle2Icon,
	ClockIcon,
	DollarSignIcon,
	KeyRoundIcon,
	LockIcon,
	PlayCircleIcon,
	PlusIcon,
	SendIcon,
	ShieldIcon,
	StarIcon,
	UsersRoundIcon,
	WorkflowIcon,
	type ZapIcon,
} from "lucide-react";
import Link from "next/link";
import type { IApp, IBoardListing } from "../../../lib";
import { IAppVisibility, RolePermissions } from "../../../lib";
import { formatDuration, formatRelativeTime } from "../../../lib/date";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Card } from "../../ui/card";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";
import { LockBadge, PermissionNotice } from "../permission";
import {
	EmptyHint,
	Meter,
	SectionCard,
	Sparkline,
	StateDot,
	VisibilityBadge,
} from "./dashboard-primitives";
import { GuardedAction } from "./project-identity-row";
import { type ProjectSurface, SurfacesTable } from "./surfaces-table";
import type { ProjectRunHealth } from "./use-project-runs";
import {
	type AiActStatus,
	type DashboardPermissions,
	type InspectorPanel,
	type ListingChecklistItem,
	isOnlineVisibility,
} from "./use-project-signals";

const MICRO_DOLLARS_PER_DOLLAR = 1_000_000;

function MetricTile({
	label,
	icon: Icon,
	value,
	hint,
	children,
}: Readonly<{
	label: string;
	icon: typeof ZapIcon;
	value: string;
	hint?: string;
	children?: React.ReactNode;
}>) {
	return (
		<Card className="gap-0 px-4 py-3">
			<span className="flex items-center gap-1.5 text-xs text-muted-foreground">
				<Icon className="h-3.5 w-3.5" />
				{label}
			</span>
			<span className="mt-1 text-2xl font-semibold tabular-nums tracking-tight">
				{value}
			</span>
			{hint && <span className="text-xs text-muted-foreground">{hint}</span>}
			{children}
		</Card>
	);
}

/**
 * Model spend comes from the analytics API, which only exists for projects
 * that are synced to a hub and is guarded by `ReadAnalytics`. Offline projects
 * fall back to a local metric rather than showing a fabricated zero; a member
 * without the permission keeps the tile but sees it locked, so the dashboard
 * does not quietly change shape from one account to the next.
 */
function useProjectSpend(
	appId: string,
	visibility: IAppVisibility,
	canRead: boolean,
) {
	const backend = useBackend();
	const analytics = backend.analyticsState;
	const applicable = !!analytics && visibility !== IAppVisibility.Offline;
	const query = useQuery({
		queryKey: ["project-spend", appId],
		enabled: !!appId && applicable && canRead,
		staleTime: 5 * 60 * 1000,
		retry: false,
		queryFn: () => {
			if (!analytics) throw new Error("Analytics unavailable");
			return analytics.getAnalyticsOverview(appId);
		},
	});

	const micros = query.data
		? query.data.totalLlmCost + query.data.totalEmbeddingCost
		: Number.NaN;
	const dollars = Number.isFinite(micros)
		? micros / MICRO_DOLLARS_PER_DOLLAR
		: null;

	return {
		applicable,
		locked: applicable && !canRead,
		isLoading: query.isLoading,
		isError: query.isError,
		dollars,
	};
}

/**
 * The operations dashboard. A project that already runs is asked about every
 * day for one reason — is it healthy, and is anything broken — so behaviour
 * leads and configuration lives behind the inspector.
 */
export function MissionControl({
	appId,
	app,
	boards,
	surfaces,
	runs,
	aiAct,
	listing,
	listingDone,
	permissions,
	onOpenPanel,
}: Readonly<{
	appId: string;
	app: IApp;
	boards: IBoardListing[];
	surfaces: ProjectSurface[];
	runs: ProjectRunHealth;
	aiAct: AiActStatus;
	listing: ListingChecklistItem[];
	listingDone: number;
	permissions: DashboardPermissions;
	onOpenPanel: (panel: InspectorPanel) => void;
}>) {
	const { t } = useTranslation("settings");
	const spend = useProjectSpend(
		appId,
		app.visibility,
		permissions.canReadAnalytics,
	);
	const listed = isOnlineVisibility(app.visibility);
	const ownerOnly = t(
		"onlyAnOwnerCanChangeThis",
		"Only an owner can change this.",
	);

	const spendTile = spend.applicable ? (
		spend.locked ? (
			<MetricTile
				label={t("modelSpend", "Model spend")}
				icon={LockIcon}
				value="—"
				hint={t("needsAnalyticsAccess", "needs analytics access")}
			/>
		) : (
			<MetricTile
				label={t("modelSpend", "Model spend")}
				icon={DollarSignIcon}
				value={spend.dollars === null ? "—" : `$${spend.dollars.toFixed(2)}`}
				hint={
					spend.dollars !== null
						? t("allTimeLlmEmbeddings", "all time, LLM + embeddings")
						: spend.isError
							? t("spendCouldNotBeRead", "spend could not be read")
							: t("loading", "loading…")
				}
			/>
		)
	) : runs.denied ? null : (
		<MetricTile
			label={t("lastRun", "Last run")}
			icon={ActivityIcon}
			value={
				runs.lastRunAt ? formatRelativeTime(runs.lastRunAt, "narrow") : "—"
			}
			hint={
				runs.lastRunAt
					? t("mostRecentExecution", "most recent execution")
					: t("neverRun", "never run")
			}
		/>
	);

	return (
		<div className="space-y-4">
			<div className="grid grid-cols-1 gap-3 min-[400px]:grid-cols-2 lg:grid-cols-4">
				{runs.denied ? (
					<PermissionNotice
						className={cn(
							"min-[400px]:col-span-2",
							spendTile ? "lg:col-span-3" : "lg:col-span-4",
						)}
						title={t("runHealthIsHidden", "Run health is hidden")}
						description={t(
							"yourRoleCannotReadThisProjectsFlowsSoRunVolumeSuccessRateAndDurationAreUnavailable",
							"Your role cannot read this project's flows, so run volume, success rate and duration are unavailable — they are not zero.",
						)}
						missing={[RolePermissions.ReadBoards]}
					/>
				) : (
					<>
						<MetricTile
							label={t("runs24h2", "Runs · 24h")}
							icon={PlayCircleIcon}
							value={runs.windowRuns.toLocaleString()}
							hint={
								runs.windowFailed > 0
									? t("windowfailedFailed", "{{windowFailed}} failed", {
											windowFailed: runs.windowFailed,
										})
									: runs.windowRuns > 0
										? t("allSucceeded", "all succeeded")
										: t("noRunsInTheLastDay", "no runs in the last day")
							}
						>
							{runs.windowRuns > 0 && (
								<Sparkline
									values={runs.trend}
									tone={runs.windowFailed > 0 ? "warn" : "ok"}
									className="mt-1"
								/>
							)}
						</MetricTile>

						<MetricTile
							label={t("successRate", "Success rate")}
							icon={CheckCircle2Icon}
							value={
								runs.successRate === null
									? "—"
									: `${runs.successRate.toFixed(1)}%`
							}
							hint={
								runs.successRate === null
									? t("needsARunToMeasure", "needs a run to measure")
									: t("valOfWindowrunsOk", "{{val}} of {{windowRuns}} ok", {
											val: runs.windowRuns - runs.windowFailed,
											windowRuns: runs.windowRuns,
										})
							}
						/>

						<MetricTile
							label={t("p95Duration", "p95 duration")}
							icon={ClockIcon}
							value={
								runs.p95Micros === null ? "—" : formatDuration(runs.p95Micros)
							}
							hint={
								runs.p95Micros === null
									? t("noRunsYet", "no runs yet")
									: t("slowest5OfRuns", "slowest 5% of runs")
							}
						/>
					</>
				)}

				{spendTile}
			</div>

			<div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1.8fr)_minmax(0,1fr)]">
				<div className="space-y-4">
					<SurfacesTable
						appId={appId}
						surfaces={surfaces}
						limit={6}
						permissions={permissions}
					/>

					<SectionCard
						title={t("flows", "Flows")}
						icon={WorkflowIcon}
						count={permissions.canReadBoards ? boards.length : undefined}
						contentClassName="p-2"
						action={
							permissions.canReadBoards && (
								<Link href={`/library/config/flows?id=${appId}`}>
									<Button variant="ghost" size="sm" className="gap-1 text-xs">
										{t("viewAll", "View all")}
										<ArrowRightIcon className="h-3 w-3" />
									</Button>
								</Link>
							)
						}
					>
						{!permissions.canReadBoards ? (
							<PermissionNotice
								title={t("flowsAreHidden", "Flows are hidden")}
								description={t(
									"yourRoleCannotListThisProjectsFlowsThisIsNotAnEmptyProject",
									"Your role cannot list this project's flows. This is not an empty project.",
								)}
								missing={[RolePermissions.ReadBoards]}
							/>
						) : boards.length === 0 ? (
							<EmptyHint>
								{t("noFlowsYet", "No flows yet.")}{" "}
								{permissions.canWriteBoards && (
									<Link
										href={`/library/config/flows?id=${appId}`}
										className="text-primary hover:underline"
									>
										{t("createYourFirstFlow", "Create your first flow")}
									</Link>
								)}
							</EmptyHint>
						) : (
							<div className="space-y-0.5">
								{boards.slice(0, 5).map((board) => {
									const health = runs.byBoard.get(board.id);
									const nodeCount = board.nodeCount;
									return (
										<Link
											key={board.id}
											href={`/flow?id=${board.id}&app=${appId}`}
											className="group flex items-center gap-3 rounded-md px-2 py-2 transition-colors hover:bg-muted/60"
										>
											<StateDot
												tone={
													!health
														? "idle"
														: health.failed > 0
															? "critical"
															: "ok"
												}
											/>
											<span className="min-w-0 flex-1">
												<span className="block truncate text-sm font-medium">
													{board.name}
												</span>
												{board.description && (
													<span className="block truncate text-xs text-muted-foreground">
														{board.description}
													</span>
												)}
											</span>
											<span className="flex shrink-0 items-center gap-3 text-xs text-muted-foreground">
												{health && health.failed > 0 && (
													<span className="text-destructive">
														{t("failedFailing", "{{failed}} failing", {
															failed: health.failed,
														})}
													</span>
												)}
												<span>
													{t("countNodes", {
														defaultValue_one: "{{count}} Node",
														defaultValue_other: "{{count}} Nodes",
														count: nodeCount,
													})}
												</span>
												{board.updatedAt && (
													<span className="hidden md:inline">
														{formatRelativeTime(board.updatedAt, "narrow")}
													</span>
												)}
												<ArrowRightIcon className="h-3 w-3 opacity-0 transition-opacity group-hover:opacity-100" />
											</span>
										</Link>
									);
								})}
								{boards.length > 5 && (
									<p className="pt-1 text-center text-xs text-muted-foreground">
										+{boards.length - 5} {t("moreFlows", "more flows")}
									</p>
								)}
							</div>
						)}
					</SectionCard>
				</div>

				<div className="space-y-4">
					<SectionCard
						title={t("liveActivity", "Live activity")}
						icon={ActivityIcon}
						contentClassName="p-2"
					>
						{runs.denied ? (
							<PermissionNotice
								title={t("activityIsHidden", "Activity is hidden")}
								description={t(
									"theRunLogIsPartOfThisProjectsFlowsWhichYourRoleCannotRead",
									"The run log is part of this project's flows, which your role cannot read.",
								)}
								missing={[RolePermissions.ReadBoards]}
							/>
						) : runs.recent.length === 0 ? (
							<EmptyHint>
								{t("noRunsRecordedYet", "No runs recorded yet.")}
							</EmptyHint>
						) : (
							<div className="space-y-0.5">
								{runs.recent.slice(0, 8).map((run) => (
									<div
										key={run.runId}
										className={cn(
											"flex items-center gap-2 rounded-md px-2 py-1.5 text-xs",
											run.failed && "bg-destructive/10",
										)}
									>
										<span className="w-10 shrink-0 tabular-nums text-muted-foreground">
											{new Date(run.startedAt).toLocaleTimeString(undefined, {
												hour: "2-digit",
												minute: "2-digit",
											})}
										</span>
										<StateDot
											tone={
												run.failed ? "critical" : run.warned ? "warn" : "ok"
											}
										/>
										<span className="min-w-0 flex-1 truncate">
											{run.boardName}
										</span>
										<span
											className={cn(
												"shrink-0 tabular-nums text-muted-foreground",
												run.failed && "text-destructive",
											)}
										>
											{run.failed
												? "failed"
												: formatDuration(run.durationMicros)}
										</span>
									</div>
								))}
							</div>
						)}
					</SectionCard>

					<SectionCard
						title={t("access", "Access")}
						icon={ShieldIcon}
						action={
							<GuardedAction
								allowed={permissions.canWriteApp}
								reason={ownerOnly}
							>
								<Button
									variant="ghost"
									size="sm"
									className="gap-1 text-xs"
									disabled={!permissions.canWriteApp}
									onClick={() => onOpenPanel("access")}
								>
									{t("edit", "Edit")}
									<ArrowRightIcon className="h-3 w-3" />
								</Button>
							</GuardedAction>
						}
					>
						<div className="space-y-2.5 text-xs">
							<div className="flex items-center gap-2">
								<span className="text-muted-foreground">
									{t("visibility", "Visibility")}
								</span>
								<span className="ml-auto">
									<VisibilityBadge visibility={app.visibility} />
								</span>
							</div>
							<div className="flex items-center gap-2">
								<UsersRoundIcon className="h-3.5 w-3.5 text-muted-foreground" />
								<span className="text-muted-foreground">
									{t("teamRoles", "Team & Roles")}
								</span>
								<span className="ml-auto">
									{!listed ? (
										<LockBadge kind="visibility">
											{t("needsPrototype", "Needs Prototype")}
										</LockBadge>
									) : permissions.canReadTeam ? (
										<Link
											href={`/library/config/team?id=${appId}`}
											className="text-primary hover:underline"
										>
											{t("manage", "Manage")}
										</Link>
									) : (
										<Tooltip>
											<TooltipTrigger asChild>
												<span>
													<LockBadge kind="permission">
														{t("locked", "Locked")}
													</LockBadge>
												</span>
											</TooltipTrigger>
											<TooltipContent side="bottom">
												{t(
													"yourRoleCannotSeeThisProjectsMembers",
													"Your role cannot see this project's members.",
												)}
											</TooltipContent>
										</Tooltip>
									)}
								</span>
							</div>
							<div className="flex items-center gap-2">
								<span className="text-muted-foreground">
									{t("forking2", "Forking")}
								</span>
								<span className="ml-auto text-foreground">
									{app.allow_forking ? "Allowed" : "Off"}
								</span>
							</div>
							{app.visibility === IAppVisibility.Private &&
								permissions.canWriteApp && (
									<Button
										variant="outline"
										size="sm"
										className="w-full"
										onClick={() => onOpenPanel("access")}
									>
										<KeyRoundIcon className="size-3.5" />
										{t(
											"switchToPrototypeToUnlockTeamFeatures",
											"Switch to Prototype to unlock team features",
										)}
									</Button>
								)}
						</div>
					</SectionCard>

					<SectionCard
						title="Publishing"
						icon={SendIcon}
						action={
							<Badge variant="outline" className="text-[10px]">
								{listed ? "Listed" : "Not listed"}
							</Badge>
						}
					>
						<div className="space-y-3 text-xs">
							<div>
								<div className="mb-1 flex items-center gap-2">
									<span className="text-muted-foreground">
										{t("storeReadiness", "Store readiness")}
									</span>
									<span className="ml-auto tabular-nums text-muted-foreground">{`${listingDone} / ${listing.length}`}</span>
								</div>
								<Meter value={listingDone} total={listing.length} />
							</div>

							{aiAct.available && (
								<button
									type="button"
									className="flex w-full items-center gap-2"
									onClick={() => onOpenPanel("compliance")}
								>
									<ShieldIcon className="h-3.5 w-3.5 text-muted-foreground" />
									<span className="text-muted-foreground">
										{t("euAiAct", "EU AI Act")}
									</span>
									<span className="ml-auto">
										{aiAct.hasAssessment ? (
											<Badge variant="secondary" className="text-[10px]">
												{aiAct.riskCategory ?? "Assessed"}
												{aiAct.conformityScore !== null &&
													` · ${aiAct.conformityScore}`}
											</Badge>
										) : (
											<Badge variant="outline" className="text-[10px]">
												{t("notSubmitted", "Not submitted")}
											</Badge>
										)}
									</span>
								</button>
							)}

							{listed ? (
								<div className="grid grid-cols-3 gap-2 pt-1">
									<div>
										<div className="text-base font-semibold tabular-nums">
											{(app.download_count ?? 0).toLocaleString()}
										</div>
										<div className="text-[11px] text-muted-foreground">
											{t("downloads", "Downloads")}
										</div>
									</div>
									<div>
										<div className="text-base font-semibold tabular-nums">
											{(app.interactions_count ?? 0).toLocaleString()}
										</div>
										<div className="text-[11px] text-muted-foreground">
											{t("interactions", "Interactions")}
										</div>
									</div>
									<div>
										<div className="flex items-center gap-1 text-base font-semibold tabular-nums">
											{app.avg_rating ? app.avg_rating.toFixed(1) : "—"}
											{app.avg_rating ? (
												<StarIcon className="h-3 w-3 text-amber-500" />
											) : null}
										</div>
										<div className="text-[11px] text-muted-foreground">
											{t("rating_countRatings", "{{rating_count}} ratings", {
												rating_count: app.rating_count,
											})}
										</div>
									</div>
								</div>
							) : (
								<p className="text-[11px] leading-relaxed text-muted-foreground">
									{t(
										"downloadsRatingsAndRevenueAppearHereOnceTheAppIsListedNotBefore",
										"Downloads, ratings and revenue appear here once the app is listed — not before.",
									)}
								</p>
							)}
						</div>
					</SectionCard>

					{permissions.canWriteBoards && (
						<Link href={`/library/config/flows?id=${appId}`}>
							<Button variant="outline" size="sm" className="w-full">
								<PlusIcon className="mr-1.5 h-3 w-3" />
								{t("newFlow", "New flow")}
							</Button>
						</Link>
					)}
				</div>
			</div>
		</div>
	);
}
