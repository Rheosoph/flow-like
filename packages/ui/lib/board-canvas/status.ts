import {
	type NodeRunState,
	nodeExecutionState,
} from "../../components/flow/flow-node/execution-state";
import {
	type ActivityStatus,
	formatTimeSinceUpdate,
	getActivityStatus,
} from "../../hooks/use-run-activity";
import {
	type IBoardHeatmap,
	nodeLogCounts,
} from "../../state/log-aggregation-state";
import type { IQualityMark, IQualitySeverity } from "../board-quality";
import type { ILogMetadata } from "../schema/flow/log-metadata";
import type { IRunLogSummary } from "../schema/flow/log-query";
import type { FlowSceneNode } from "./scene";

export type StatusSeverity = IQualitySeverity | "fatal";

/** What the canvas paints for one node's runs, logs and lint marks. Absent fields are off. */
export interface NodeStatus {
	run?: "running" | "done";
	runId?: string;
	/** The current run's log count; an empty badge marks a node that ran without a summary. */
	logs?: { count?: string; problems?: boolean; warnings?: boolean };
	/** The worst level the node logged in the current run, from warning up. */
	severity?: StatusSeverity;
	heat?: { intensity: number; visits: string; errors?: string };
	quality?: { severity: IQualitySeverity; count?: string };
	/** A collaborator's run is executing the node. */
	remote?: boolean;
}

export interface RunActivity {
	text: string;
	tone: ActivityStatus;
}

export interface StatusSources {
	runs?: ReadonlyMap<string, NodeRunState>;
	currentMetadata?: ILogMetadata;
	currentSummary?: IRunLogSummary;
	currentSummaryRunId?: string;
	heatmapEnabled?: boolean;
	heatmap?: IBoardHeatmap;
	qualityInline?: boolean;
	qualityMarks?: Record<string, Record<string, IQualityMark>>;
}

const compact = new Intl.NumberFormat(undefined, {
	notation: "compact",
	maximumFractionDigits: 1,
});

/** Log levels as numbered in run metadata: debug, info, warn, error, fatal. */
const RUN_SEVERITY: Record<number, StatusSeverity> = {
	2: "warning",
	3: "error",
	4: "fatal",
};

function logBadge(
	sources: StatusSources,
	nodeId: string,
	executed: boolean,
): NodeStatus["logs"] {
	const counts = nodeLogCounts(sources, nodeId);
	if (counts && counts.total > 0) {
		const problems = counts.problems > 0;
		return {
			count: compact.format(problems ? counts.problems : counts.total),
			problems,
			warnings: !problems && counts.warnings > 0,
		};
	}
	return executed ? {} : undefined;
}

function heat(
	sources: StatusSources,
	nodeId: string,
): NodeStatus["heat"] | undefined {
	if (!sources.heatmapEnabled || !sources.heatmap) return undefined;
	const visits = sources.heatmap.nodes[nodeId];
	if (!visits || visits.visits <= 0) return undefined;
	return {
		intensity:
			sources.heatmap.maxVisits > 0
				? visits.visits / sources.heatmap.maxVisits
				: 0,
		visits: `${visits.visits}×`,
		errors: visits.errors > 0 ? `${visits.errors}!` : undefined,
	};
}

function quality(
	sources: StatusSources,
	node: FlowSceneNode,
): NodeStatus["quality"] | undefined {
	const boardId = node.data.boardId;
	if (!sources.qualityInline || typeof boardId !== "string") return undefined;
	const mark = sources.qualityMarks?.[boardId]?.[node.id];
	if (!mark) return undefined;
	const total = mark.counts.error + mark.counts.warning + mark.counts.info;
	return {
		severity: mark.severity,
		count: total > 1 ? String(total) : undefined,
	};
}

function statusOf(
	node: FlowSceneNode,
	sources: StatusSources,
	level: number | undefined,
): NodeStatus | undefined {
	const run = sources.runs && nodeExecutionState(sources.runs, node.id);
	const live = run?.status === "none" ? undefined : run;
	const fields = Object.entries({
		run: live?.status,
		runId: live?.runId,
		logs: logBadge(sources, node.id, level !== undefined),
		severity: level === undefined ? undefined : RUN_SEVERITY[level],
		heat: heat(sources, node.id),
		quality: quality(sources, node),
		remote: node.data.remoteExecuting === true || undefined,
	}).filter(([, value]) => value !== undefined);
	return fields.length > 0
		? (Object.fromEntries(fields) as NodeStatus)
		: undefined;
}

/**
 * The run, log, lint and collaboration state FlowNode shows, for the canvas to paint without
 * mounting the node. Only nodes with something to show are listed.
 */
export function collectNodeStatus(
	nodes: readonly FlowSceneNode[],
	sources: StatusSources,
): Record<string, NodeStatus> {
	const logged = new Map<string, number>();
	for (const [nodeId, level] of sources.currentMetadata?.nodes ?? []) {
		if (typeof nodeId === "string")
			logged.set(nodeId, typeof level === "number" ? level : 0);
	}
	const result: Record<string, NodeStatus> = {};
	for (const node of nodes) {
		const status = statusOf(node, sources, logged.get(node.id));
		if (status) result[node.id] = status;
	}
	return result;
}

/** Each run's "last activity" text, as FlowNode shows it on its running nodes. */
export function collectRunActivity(
	runs: ReadonlyMap<string, { lastNodeUpdateMs: number }>,
	now: number,
): Record<string, RunActivity> {
	const activity: Record<string, RunActivity> = {};
	for (const [runId, run] of runs) {
		const since = Math.max(0, now - run.lastNodeUpdateMs);
		activity[runId] = {
			text: formatTimeSinceUpdate(since),
			tone: getActivityStatus(since),
		};
	}
	return activity;
}
