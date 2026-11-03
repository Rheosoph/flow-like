"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import {
	AlertTriangleIcon,
	BanIcon,
	CheckCircle2Icon,
	CircleXIcon,
	CloudIcon,
	CopyIcon,
	HardDriveIcon,
	InfoIcon,
	Loader2Icon,
	ScrollIcon,
} from "lucide-react";
import { type ReactElement, useEffect, useId, useRef, useState } from "react";
import { toast } from "sonner";
import { useAppPermissions } from "../../../hooks/use-app-permissions";
import { formatDuration, formatRelativeTime } from "../../../lib/date";
import type { aggregateNodeSeverity } from "../../../lib/event-history";
import { RolePermissions } from "../../../lib/permission/role-permission";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import type { IEventTimelineRun } from "../../../state/backend-state/event-state";
import { runKey } from "../../flow/logs/use-log-window";
import { Button, Tooltip, TooltipContent, TooltipTrigger } from "../../ui";
import { PermissionNotice } from "../permission/permission-notice";

export function HistoryRuns({
	runs,
	isLoading,
	runsSupported,
	runsLocked,
	runName,
	onOpenRun,
	nodeId,
}: Readonly<{
	runs: IEventTimelineRun[];
	isLoading: boolean;
	runsSupported: boolean;
	runsLocked: boolean;
	runName: (run: IEventTimelineRun) => string;
	onOpenRun: (run: IEventTimelineRun) => void;
	nodeId?: string;
}>) {
	const { t } = useTranslation("settings");

	if (runsLocked) {
		return (
			<PermissionNotice
				title={t("runHistoryUnavailable", "Run history unavailable")}
				description={t(
					"readingAnEventsRunsAlsoNeedsWorkflowAccess",
					"Runs are read through the flow they executed, so listing them also needs permission to view this project's flows.",
				)}
				missing={[RolePermissions.ReadBoards]}
			/>
		);
	}
	if (!runsSupported) {
		return (
			<p className="py-6 text-center text-sm text-muted-foreground">
				{t(
					"eventRunHistoryNotAvailableHere",
					"Run history isn't available on this platform yet. Only versions are shown.",
				)}
			</p>
		);
	}
	if (isLoading) {
		return (
			<div className="flex flex-col items-center justify-center gap-2 py-8">
				<Loader2Icon className="h-6 w-6 animate-spin text-muted-foreground" />
				<p className="text-sm text-muted-foreground">
					{t("loadingRuns", "Loading runs...")}
				</p>
			</div>
		);
	}
	if (runs.length === 0) {
		return (
			<div className="flex flex-col items-center justify-center gap-1 py-8 text-center">
				<ScrollIcon className="size-5 text-muted-foreground/60" />
				<p className="text-sm font-medium">{t("noRuns", "No runs")}</p>
				<p className="text-xs text-muted-foreground">
					{t(
						"noRunsRecordedForThisEventYet",
						"No runs have been recorded for this event yet.",
					)}
				</p>
			</div>
		);
	}

	return (
		<div className="flex max-h-105 flex-col gap-2 overflow-y-auto">
			{runs.map((run) => (
				<HistoryRunRow
					key={`${run.board_id}/${run.run_id}`}
					run={run}
					name={runName(run)}
					nodeId={nodeId}
					onOpen={() => onOpenRun(run)}
				/>
			))}
		</div>
	);
}

/** Load previews only as rows enter the scroll viewport. */
function useVisibleRow() {
	const ref = useRef<HTMLDivElement>(null);
	const [visible, setVisible] = useState(false);
	useEffect(() => {
		if (!ref.current || typeof IntersectionObserver === "undefined") {
			setVisible(true);
			return;
		}
		const observer = new IntersectionObserver((entries) => {
			if (entries.some((entry) => entry.isIntersecting)) {
				setVisible(true);
				observer.disconnect();
			}
		});
		observer.observe(ref.current);
		return () => observer.disconnect();
	}, []);
	return {
		ref,
		visible,
		onFocus: () => setVisible(true),
		onPointerEnter: () => setVisible(true),
	};
}

function useRunProblem(
	run: IEventTimelineRun | undefined,
	visible: boolean,
	nodeId?: string,
) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const permissions = useAppPermissions(
		run?.is_remote ? run.app_id : undefined,
	);
	const readable =
		!run?.is_remote || permissions.canStrict(RolePermissions.ReadLogs);
	const level = nodeId
		? Number(run?.nodes?.find(([id]) => id === nodeId)?.[1] ?? 0)
		: (run?.log_level ?? 0);
	const problem = useQuery({
		queryKey: [
			"event-history-problem",
			run ? runKey(run) : "",
			run?.is_remote === true,
			nodeId ?? "",
		],
		queryFn: async () => {
			if (!run) return null;
			const logs = await backend.boardState.queryRunLogs(
				run,
				{ levels: [2, 3, 4], ...(nodeId ? { nodes: [nodeId] } : {}) },
				0,
				1,
			);
			return logs[0]?.message ?? null;
		},
		enabled: !!run && level >= 2 && readable && visible,
		retry: false,
		staleTime: 60_000,
		gcTime: 5 * 60_000,
		meta: { persist: false },
	});
	// A run-wide failure cannot be attributed to any particular visited node.
	const message =
		(readable ? problem.data : undefined) ||
		(!nodeId ? run?.error_message : undefined);
	const hint =
		message ||
		(!readable
			? t(
					"historyLogPermissionHint",
					"Reading this run's messages needs Logs permission.",
				)
			: problem.isError
				? t(
						"historyProblemLoadFailed",
						"Could not load the warning or error message. Open the run to retry.",
					)
				: problem.isFetching || !visible
					? t("loadingRunMessage", "Loading run message…")
					: t(
							"noRunProblemMessage",
							"No warning or error message was recorded.",
						));
	return { message, hint, level };
}

function RunSeverityTooltip({
	level,
	message,
	name,
	children,
}: Readonly<{
	level: number;
	message?: string;
	name?: string;
	children: ReactElement;
}>) {
	const { t } = useTranslation("settings");
	const descriptionId = useId();
	const label =
		level >= 4
			? t("runFatal", "Fatal")
			: level === 3
				? t("runError", "Error")
				: level === 2
					? t("runWarning", "Warning")
					: t("runSuccess", "Success");
	const description = message && level >= 2 ? `${label}: ${message}` : label;
	return (
		<>
			<Tooltip>
				<TooltipTrigger asChild aria-describedby={descriptionId}>
					{children}
				</TooltipTrigger>
				<TooltipContent className="max-w-sm whitespace-pre-wrap break-words">
					{description}
				</TooltipContent>
			</Tooltip>
			<span id={descriptionId} className="sr-only">
				{name ? `${name}. ${description}` : description}
			</span>
		</>
	);
}

function RunSeverityIcon({ level }: Readonly<{ level: number }>) {
	const Icon =
		level >= 4
			? BanIcon
			: level === 3
				? CircleXIcon
				: level === 2
					? AlertTriangleIcon
					: CheckCircle2Icon;
	return (
		<Icon
			aria-hidden
			className={cn(
				"h-3 w-3 shrink-0",
				level >= 3
					? "text-destructive"
					: level === 2
						? "text-yellow-500"
						: "text-green-500",
			)}
		/>
	);
}

function ProblemLine({
	message,
	level,
}: Readonly<{ message?: string | null; level: number }>) {
	if (!message) return null;
	return (
		<span
			title={message}
			className={cn(
				"block truncate text-xs",
				level >= 3
					? "text-destructive"
					: "text-yellow-600 dark:text-yellow-400",
			)}
		>
			{message}
		</span>
	);
}

function HistoryRunRow({
	run,
	name,
	nodeId,
	onOpen,
}: Readonly<{
	run: IEventTimelineRun;
	name: string;
	nodeId?: string;
	onOpen: () => void;
}>) {
	const { t } = useTranslation("settings");
	const { visible, ...row } = useVisibleRow();
	const problem = useRunProblem(run, visible, nodeId);
	return (
		<div
			{...row}
			className="flex items-center gap-2 rounded-md border p-2 hover:bg-muted/40 focus-within:ring-2 focus-within:ring-ring"
		>
			<RunSeverityTooltip
				level={problem.level}
				message={problem.hint}
				name={name}
			>
				<button
					type="button"
					aria-label={t("openHistoryRun", "Open run {{id}}", {
						id: run.run_id,
					})}
					onClick={onOpen}
					className="flex min-w-0 flex-1 items-center justify-between gap-3 rounded-sm text-left focus-visible:outline-none"
				>
					<span className="flex min-w-0 flex-1 flex-col gap-2">
						<span className="flex min-w-0 flex-wrap items-center gap-2">
							{run.is_remote ? (
								<CloudIcon
									aria-label={t("remoteExecution", "Remote execution")}
									className="h-3 w-3 text-blue-500"
								/>
							) : (
								<HardDriveIcon
									aria-label={t("localExecution", "Local execution")}
									className="h-3 w-3 text-muted-foreground"
								/>
							)}
							<span className="truncate text-sm">{name}</span>
							<small className="font-mono text-muted-foreground">
								{run.event_version
									? `v${run.event_version}`
									: t("unversioned", "Unversioned")}
							</small>
							<small className="text-muted-foreground">{run.version}</small>
							{run.actor?.kind === "bot" && (
								<small className="text-muted-foreground">
									{t("runByBot", "by {{name}}", {
										name: run.actor.name || run.actor.id,
									})}
								</small>
							)}
						</span>
						<span className="flex min-w-0 flex-wrap gap-x-3 gap-y-1 text-xs text-muted-foreground">
							<time
								dateTime={new Date(Math.floor(run.start / 1000)).toISOString()}
								title={new Date(Math.floor(run.start / 1000)).toLocaleString()}
							>
								{formatRelativeTime(Math.floor(run.start / 1000), "narrow")}
							</time>
							<span className="truncate font-mono" title={run.run_id}>
								{t("historyRunId", "Run ID")}: {run.run_id}
							</span>
						</span>
						<ProblemLine message={problem.message} level={problem.level} />
					</span>
					<span className="flex shrink-0 items-center gap-2">
						<small className="text-muted-foreground">
							{formatDuration(Math.abs(run.end - run.start))}
						</small>
						<RunSeverityIcon level={problem.level} />
					</span>
				</button>
			</RunSeverityTooltip>
			<Button
				type="button"
				variant="ghost"
				size="icon"
				className="size-7 shrink-0"
				aria-label={t("copyRunId", "Copy run ID")}
				title={t("copyRunId", "Copy run ID")}
				onClick={async () => {
					try {
						await navigator.clipboard.writeText(run.run_id);
						toast.success(t("runIdCopied", "Run ID copied"));
					} catch {
						toast.error(t("runIdCopyFailed", "Could not copy the run ID"));
					}
				}}
			>
				<CopyIcon className="size-3.5" />
			</Button>
		</div>
	);
}

export function HistoryNodes({
	runs,
	aggregates,
	nodeName,
	onSelectNode,
}: Readonly<{
	runs: IEventTimelineRun[];
	aggregates: ReturnType<typeof aggregateNodeSeverity>;
	nodeName: (nodeId: string) => string;
	onSelectNode: (nodeId: string) => void;
}>) {
	const { t } = useTranslation("settings");
	return (
		<div className="space-y-3">
			<div className="flex gap-3 rounded-lg border bg-muted/40 p-3">
				<InfoIcon className="mt-0.5 h-4 w-4 shrink-0 text-muted-foreground" />
				<p className="text-sm text-muted-foreground">
					{t(
						"nodeTimingsNeedDebugLevel",
						"Visit counts and severity are recorded at every log level. Per-node timings only exist when the flow logs at Debug level, so they are not shown here.",
					)}
				</p>
			</div>
			{aggregates.length === 0 ? (
				<p className="py-4 text-center text-sm text-muted-foreground">
					{t(
						"noNodeActivityRecorded",
						"No node activity recorded in the loaded runs.",
					)}
				</p>
			) : (
				<div className="flex max-h-90 flex-col gap-1.5 overflow-y-auto">
					{aggregates.map((node) => (
						<HistoryNodeRow
							key={node.nodeId}
							node={node}
							name={nodeName(node.nodeId)}
							run={runs.find((run) =>
								run.nodes?.some(
									([id, level]) =>
										id === node.nodeId &&
										node.worstLevel >= 2 &&
										Number(level) === node.worstLevel,
								),
							)}
							onSelect={() => onSelectNode(node.nodeId)}
						/>
					))}
				</div>
			)}
		</div>
	);
}

function HistoryNodeRow({
	node,
	name,
	run,
	onSelect,
}: Readonly<{
	node: ReturnType<typeof aggregateNodeSeverity>[number];
	name: string;
	run?: IEventTimelineRun;
	onSelect: () => void;
}>) {
	const { t } = useTranslation("settings");
	const { visible, ...row } = useVisibleRow();
	const problem = useRunProblem(run, visible, node.nodeId);
	return (
		<div {...row}>
			<RunSeverityTooltip level={node.worstLevel} message={problem.hint}>
				<button
					type="button"
					onClick={onSelect}
					className="flex w-full items-center justify-between gap-2 rounded-md border p-2 text-left text-sm hover:bg-muted/40 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
					aria-label={t("showHistoryNodeRuns", "Show runs for {{name}}", {
						name,
					})}
				>
					<span className="flex min-w-0 flex-1 flex-col gap-1">
						<span className="flex min-w-0 items-center gap-2">
							<RunSeverityIcon level={node.worstLevel} />
							<span className="truncate" title={node.nodeId}>
								{name}
							</span>
						</span>
						<ProblemLine message={problem.message} level={problem.level} />
					</span>
					<span className="flex shrink-0 flex-wrap justify-end gap-3 text-xs text-muted-foreground">
						{node.warnRuns > 0 && (
							<span className="tabular-nums">
								{t("warnRunCount", "{{num}} warn", { num: node.warnRuns })}
							</span>
						)}
						{node.failRuns > 0 && (
							<span className="tabular-nums text-destructive">
								{t("failedRunCount", "{{num}} failed", { num: node.failRuns })}
							</span>
						)}
						<span className="tabular-nums">
							{t("nodeVisitCount", "{{num}} runs", { num: node.visits })}
						</span>
					</span>
				</button>
			</RunSeverityTooltip>
		</div>
	);
}
