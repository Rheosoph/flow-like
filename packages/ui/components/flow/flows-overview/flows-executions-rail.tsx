"use client";

import { useTranslation } from "@flow-like/locales";
import { useMemo } from "react";
import { formatDuration } from "../../../lib/date";
import type { IEvent } from "../../../lib/schema/flow/event";
import { cn } from "../../../lib/utils";
import type {
	ProjectRun,
	ProjectRunHealth,
} from "../../settings/dashboard/use-project-runs";
import { Skeleton } from "../../ui/skeleton";
import type { IFlowRow } from "./flows-overview-model";
import { RUN_RECENCY_ORDER, groupRunsByRecency } from "./flows-overview-model";

const HISTOGRAM_HEIGHT = 48;

function relativeAge(startedAt: number, now: number): string {
	const seconds = Math.max(0, Math.round((now - startedAt) / 1000));
	if (seconds < 60) return `${seconds}s`;
	const minutes = Math.round(seconds / 60);
	if (minutes < 60) return `${minutes}m`;
	const hours = Math.round(minutes / 60);
	if (hours < 24) return `${hours}h`;
	return `${Math.round(hours / 24)}d`;
}

/**
 * Total runs per two-hour bucket with the failed share stacked on top, so a bar
 * that is mostly red reads as trouble even before you reach the number above it.
 */
function RunHistogram({
	trend,
	trendFailed,
}: Readonly<{ trend: number[]; trendFailed: number[] }>) {
	const { t } = useTranslation("flow");
	const max = Math.max(1, ...trend);
	// Buckets are fixed two-hour slots, so their age is a stable identity.
	const buckets = trend.map((total, index) => ({
		hoursAgo: (trend.length - index) * 2,
		total,
		failed: trendFailed[index] ?? 0,
	}));
	return (
		<div>
			<div
				className="flex items-end gap-[3px]"
				style={{ height: `${HISTOGRAM_HEIGHT}px` }}
			>
				{buckets.map(({ hoursAgo, total, failed }) => {
					const height = (total / max) * HISTOGRAM_HEIGHT;
					const failedHeight = total > 0 ? (failed / total) * height : 0;
					return (
						<div
							key={`bucket-${hoursAgo}h`}
							className="flex h-full flex-1 flex-col justify-end gap-px"
							title={t("hoursAgoRangeCountRunsFailed", {
								defaultValue_one:
									"{{hoursAgo}}–{{rangeEnd}} h ago: {{count}} run, {{failed}} failed",
								defaultValue_other:
									"{{hoursAgo}}–{{rangeEnd}} h ago: {{count}} runs, {{failed}} failed",
								hoursAgo,
								rangeEnd: hoursAgo - 2,
								count: total,
								failed,
							})}
						>
							{total === 0 ? (
								<span className="h-px w-full rounded-full bg-border" />
							) : (
								<>
									{failed > 0 ? (
										<span
											className="w-full rounded-t-[2px] bg-red-500/80"
											style={{ height: `${Math.max(2, failedHeight)}px` }}
										/>
									) : null}
									<span
										className={cn(
											"w-full bg-primary/35",
											failed === 0 && "rounded-t-[2px]",
										)}
										style={{
											height: `${Math.max(1, height - failedHeight)}px`,
										}}
									/>
								</>
							)}
						</div>
					);
				})}
			</div>
			<div className="mt-1.5 flex justify-between text-[9px] uppercase tracking-wider text-muted-foreground/50">
				<span>{t("24HAgo", "24 h ago")}</span>
				<span>{t("2hourBuckets", "2-hour buckets")}</span>
				<span>{t("now", "Now")}</span>
			</div>
		</div>
	);
}

function RunRow({
	run,
	now,
	eventName,
	entryName,
	onSelect,
}: Readonly<{
	run: ProjectRun;
	now: number;
	eventName: string;
	entryName: string;
	onSelect: (boardId: string) => void;
}>) {
	const { t } = useTranslation("flow");
	return (
		<button
			type="button"
			onClick={() => onSelect(run.boardId)}
			className="flex w-full items-start gap-2 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-muted/60"
		>
			<span
				className={cn(
					"mt-1.5 size-1.5 shrink-0 rounded-full",
					run.failed
						? "bg-red-500"
						: run.warned
							? "bg-amber-500"
							: "bg-emerald-500/70",
				)}
			/>
			<span className="min-w-0 flex-1">
				<span className="block truncate text-xs font-semibold leading-tight">
					{run.boardName}
				</span>
				<span className="mt-0.5 block truncate text-[10px] text-muted-foreground">{`${eventName} → ${entryName}`}</span>
			</span>
			<span className="ml-auto shrink-0 text-right">
				<span className="block text-[10px] tabular-nums text-muted-foreground/70">
					{relativeAge(run.startedAt, now)}
				</span>
				<span
					className={cn(
						"block font-mono text-[10px] tabular-nums",
						run.failed
							? "text-red-500"
							: run.warned
								? "text-amber-500"
								: "text-muted-foreground",
					)}
				>
					{formatDuration(run.durationMicros)}
				</span>
			</span>
		</button>
	);
}

export interface FlowsExecutionsRailProps {
	runs: ProjectRunHealth;
	rows: IFlowRow[];
	events: IEvent[];
	onSelectRun: (boardId: string) => void;
}

export function FlowsExecutionsRail({
	runs,
	rows,
	events,
	onSelectRun,
}: Readonly<FlowsExecutionsRailProps>) {
	const { t } = useTranslation("flow");
	const eventNames = useMemo(
		() => new Map(events.map((event) => [event.id, event.name])),
		[events],
	);
	const entryNames = useMemo(() => {
		const names = new Map<string, string>();
		for (const row of rows) {
			for (const node of row.entryPoints) {
				names.set(node.id, node.friendly_name || node.name);
			}
		}
		return names;
	}, [rows]);

	// One timestamp per render so recency buckets cannot drift between rows.
	// Not memoized: `now` would be a stale dependency, and 24 runs is nothing.
	const now = Date.now();
	const grouped = groupRunsByRecency(runs.recent, now);

	return (
		<aside
			aria-label={t("recentExecutions", "Recent executions")}
			className="flex w-full shrink-0 flex-col gap-3 rounded-xl border border-border/60 bg-card/80 p-3 backdrop-blur-sm min-[1120px]:sticky min-[1120px]:top-4 min-[1120px]:w-84 dark:border-white/10 dark:bg-muted/40"
		>
			<div>
				<p className="flex items-baseline gap-2">
					<span className="text-[10px] font-semibold uppercase tracking-[0.1em] text-muted-foreground/70">
						{t("executions", "Executions")}
					</span>
					<span className="text-[10px] text-muted-foreground/60">
						{t("acrossAllFlowsLast24H", "across all flows · last 24 h")}
					</span>
				</p>
				{runs.isLoading && !runs.ready ? (
					<Skeleton className="mt-2 h-7 w-24" />
				) : (
					<p className="mt-1.5 flex items-baseline gap-2">
						<span className="font-mono text-2xl font-semibold leading-none tabular-nums">
							{runs.windowRuns.toLocaleString()}
						</span>
						<span className="text-xs text-muted-foreground">
							{t("runs", "Runs")}
						</span>
						{runs.windowFailed > 0 ? (
							<span className="ml-auto font-mono text-xs tabular-nums text-red-500">
								{t("windowfailedFailed", "{{windowFailed}} failed", {
									windowFailed: runs.windowFailed,
								})}
							</span>
						) : null}
					</p>
				)}
			</div>

			{runs.windowRuns > 0 ? (
				<RunHistogram trend={runs.trend} trendFailed={runs.trendFailed} />
			) : null}

			{runs.recent.length === 0 ? (
				<p className="rounded-md border border-dashed border-border/60 px-2 py-3 text-center text-[11px] text-muted-foreground">
					{runs.ready
						? t(
								"noFlowInThisProjectHasRunYet",
								"No flow in this project has run yet.",
							)
						: t("loadingRunHistory", "Loading run history…")}
				</p>
			) : (
				<div className="flex flex-col">
					{RUN_RECENCY_ORDER.map((bucket) => {
						const items = grouped.get(bucket) ?? [];
						if (items.length === 0) return null;
						return (
							<div key={bucket}>
								<p className="px-2 pb-1 pt-2 text-[10px] font-semibold uppercase tracking-[0.1em] text-muted-foreground/60">
									{
										{
											minutes: t("lastFewMinutes", "Last few minutes"),
											hour: t("pastHour", "Past hour"),
											today: t("earlierToday", "Earlier today"),
											older: t("olderThanADay", "Older than a day"),
										}[bucket]
									}
								</p>
								{items.map((run) => (
									<RunRow
										key={run.runId}
										run={run}
										now={now}
										eventName={
											eventNames.get(run.eventId) ??
											t("directRun", "Direct run")
										}
										entryName={entryNames.get(run.nodeId) ?? "unknown node"}
										onSelect={onSelectRun}
									/>
								))}
							</div>
						);
					})}
				</div>
			)}

			<p className="border-t border-border/50 pt-2 text-[10px] leading-relaxed text-muted-foreground/60">
				{t(
					"readFromTheRunLogUpTo200RunsEachAcrossAtMost25FlowsRefreshedEveryMinuteOutcomeComesFromEachRunsLogLevelThereIsNoSeparateStatusField",
					"Read from the run log — up to 200 runs each across at most 25 Flows, refreshed every minute. Outcome comes from each run's log level; there is no separate status field.",
				)}
			</p>
		</aside>
	);
}
