"use client";

import { useQuery } from "@tanstack/react-query";
import {
	ActivityIcon,
	ArrowRightIcon,
	CheckCircle2Icon,
	ClockIcon,
	DollarSignIcon,
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
import type { IApp, IBoard } from "../../../lib";
import { IAppVisibility } from "../../../lib";
import { formatDuration, formatRelativeTime } from "../../../lib/date";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import { Card } from "../../ui/card";
import {
	EmptyHint,
	Meter,
	SectionCard,
	Sparkline,
	StateDot,
	VisibilityBadge,
} from "./dashboard-primitives";
import { type ProjectSurface, SurfacesTable } from "./surfaces-table";
import type { ProjectRunHealth } from "./use-project-runs";
import {
	type AiActStatus,
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
 * that are synced to a hub. Offline projects fall back to a local metric
 * rather than showing a fabricated zero.
 */
function useProjectSpend(appId: string, visibility: IAppVisibility) {
	const backend = useBackend();
	const analytics = backend.analyticsState;
	return useQuery({
		queryKey: ["project-spend", appId],
		enabled: !!appId && !!analytics && visibility !== IAppVisibility.Offline,
		staleTime: 5 * 60 * 1000,
		retry: false,
		queryFn: () => {
			if (!analytics) throw new Error("Analytics unavailable");
			return analytics.getAnalyticsOverview(appId);
		},
	});
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
	onOpenPanel,
}: Readonly<{
	appId: string;
	app: IApp;
	boards: IBoard[];
	surfaces: ProjectSurface[];
	runs: ProjectRunHealth;
	aiAct: AiActStatus;
	listing: ListingChecklistItem[];
	listingDone: number;
	onOpenPanel: (panel: InspectorPanel) => void;
}>) {
	const spend = useProjectSpend(appId, app.visibility);
	const listed = isOnlineVisibility(app.visibility);
	const spendDollars =
		spend.data != null
			? (spend.data.totalLlmCost + spend.data.totalEmbeddingCost) /
				MICRO_DOLLARS_PER_DOLLAR
			: null;

	return (
		<div className="space-y-4">
			<div className="grid grid-cols-2 gap-3 lg:grid-cols-4">
				<MetricTile
					label="Runs · 24h"
					icon={PlayCircleIcon}
					value={runs.windowRuns.toLocaleString()}
					hint={
						runs.windowFailed > 0
							? `${runs.windowFailed} failed`
							: runs.windowRuns > 0
								? "all succeeded"
								: "no runs in the last day"
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
					label="Success rate"
					icon={CheckCircle2Icon}
					value={
						runs.successRate === null ? "—" : `${runs.successRate.toFixed(1)}%`
					}
					hint={
						runs.successRate === null
							? "needs a run to measure"
							: `${runs.windowRuns - runs.windowFailed} of ${runs.windowRuns} ok`
					}
				/>

				<MetricTile
					label="p95 duration"
					icon={ClockIcon}
					value={runs.p95Micros === null ? "—" : formatDuration(runs.p95Micros)}
					hint={runs.p95Micros === null ? "no runs yet" : "slowest 5% of runs"}
				/>

				{spendDollars !== null ? (
					<MetricTile
						label="Model spend"
						icon={DollarSignIcon}
						value={`$${spendDollars.toFixed(2)}`}
						hint="all time, LLM + embeddings"
					/>
				) : (
					<MetricTile
						label="Last run"
						icon={ActivityIcon}
						value={
							runs.lastRunAt
								? formatRelativeTime(
										new Date(runs.lastRunAt).toISOString(),
										"narrow",
									)
								: "—"
						}
						hint={runs.lastRunAt ? "most recent execution" : "never run"}
					/>
				)}
			</div>

			<div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1.8fr)_minmax(0,1fr)]">
				<div className="space-y-4">
					<SurfacesTable appId={appId} surfaces={surfaces} limit={6} />

					<SectionCard
						title="Flows"
						icon={WorkflowIcon}
						count={boards.length}
						contentClassName="p-2"
						action={
							<Link href={`/library/config/flows?id=${appId}`}>
								<Button variant="ghost" size="sm" className="gap-1 text-xs">
									View all
									<ArrowRightIcon className="h-3 w-3" />
								</Button>
							</Link>
						}
					>
						{boards.length === 0 ? (
							<EmptyHint>
								No flows yet.{" "}
								<Link
									href={`/library/config/flows?id=${appId}`}
									className="text-primary hover:underline"
								>
									Create your first flow
								</Link>
							</EmptyHint>
						) : (
							<div className="space-y-0.5">
								{boards.slice(0, 5).map((board) => {
									const health = runs.byBoard.get(board.id);
									const nodeCount = Object.keys(board.nodes).length;
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
														{health.failed} failing
													</span>
												)}
												<span>{nodeCount} nodes</span>
												{board.updated_at && (
													<span className="hidden md:inline">
														{formatRelativeTime(board.updated_at, "narrow")}
													</span>
												)}
												<ArrowRightIcon className="h-3 w-3 opacity-0 transition-opacity group-hover:opacity-100" />
											</span>
										</Link>
									);
								})}
								{boards.length > 5 && (
									<p className="pt-1 text-center text-xs text-muted-foreground">
										+{boards.length - 5} more flows
									</p>
								)}
							</div>
						)}
					</SectionCard>
				</div>

				<div className="space-y-4">
					<SectionCard
						title="Live activity"
						icon={ActivityIcon}
						contentClassName="p-2"
					>
						{runs.recent.length === 0 ? (
							<EmptyHint>No runs recorded yet.</EmptyHint>
						) : (
							<div className="space-y-0.5">
								{runs.recent.map((run) => (
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
						title="Access"
						icon={ShieldIcon}
						action={
							<Button
								variant="ghost"
								size="sm"
								className="gap-1 text-xs"
								onClick={() => onOpenPanel("access")}
							>
								Edit
								<ArrowRightIcon className="h-3 w-3" />
							</Button>
						}
					>
						<div className="space-y-2.5 text-xs">
							<div className="flex items-center gap-2">
								<span className="text-muted-foreground">Visibility</span>
								<span className="ml-auto">
									<VisibilityBadge visibility={app.visibility} />
								</span>
							</div>
							<div className="flex items-center gap-2">
								<UsersRoundIcon className="h-3.5 w-3.5 text-muted-foreground" />
								<span className="text-muted-foreground">Team & Roles</span>
								<span className="ml-auto">
									{listed ? (
										<Link
											href={`/library/config/team?id=${appId}`}
											className="text-primary hover:underline"
										>
											Manage
										</Link>
									) : (
										<Badge variant="outline" className="text-[10px]">
											Needs Prototype
										</Badge>
									)}
								</span>
							</div>
							<div className="flex items-center gap-2">
								<span className="text-muted-foreground">Forking</span>
								<span className="ml-auto text-foreground">
									{app.allow_forking ? "Allowed" : "Off"}
								</span>
							</div>
							{app.visibility === IAppVisibility.Private && (
								<Button
									variant="outline"
									size="sm"
									className="w-full"
									onClick={() => onOpenPanel("access")}
								>
									Switch to Prototype to unlock team features
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
									<span className="text-muted-foreground">Store readiness</span>
									<span className="ml-auto tabular-nums text-muted-foreground">
										{listingDone} / {listing.length}
									</span>
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
									<span className="text-muted-foreground">EU AI Act</span>
									<span className="ml-auto">
										{aiAct.hasAssessment ? (
											<Badge variant="secondary" className="text-[10px]">
												{aiAct.riskCategory ?? "Assessed"}
												{aiAct.conformityScore !== null &&
													` · ${aiAct.conformityScore}`}
											</Badge>
										) : (
											<Badge variant="outline" className="text-[10px]">
												Not submitted
											</Badge>
										)}
									</span>
								</button>
							)}

							{listed ? (
								<div className="grid grid-cols-3 gap-2 pt-1">
									<div>
										<div className="text-base font-semibold tabular-nums">
											{app.download_count.toLocaleString()}
										</div>
										<div className="text-[11px] text-muted-foreground">
											Downloads
										</div>
									</div>
									<div>
										<div className="text-base font-semibold tabular-nums">
											{app.interactions_count.toLocaleString()}
										</div>
										<div className="text-[11px] text-muted-foreground">
											Interactions
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
											{app.rating_count} ratings
										</div>
									</div>
								</div>
							) : (
								<p className="text-[11px] leading-relaxed text-muted-foreground">
									Downloads, ratings and revenue appear here once the app is
									listed — not before.
								</p>
							)}
						</div>
					</SectionCard>

					<Link href={`/library/config/flows?id=${appId}`}>
						<Button variant="outline" size="sm" className="w-full">
							<PlusIcon className="mr-1.5 h-3 w-3" />
							New flow
						</Button>
					</Link>
				</div>
			</div>
		</div>
	);
}
