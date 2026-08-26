import {
	type IBoardScores,
	type IBoardWasmUsage,
	type IFlaggedPattern,
	type ILayerCounts,
	type IScoreCategory,
	type IScoreCoverage,
	type IVariableCounts,
	SCORE_CATEGORIES,
	boardVersionLabel,
	worstDimension,
	worstScore,
} from "../../../lib/board-metrics";
import { ILayerType } from "../../../lib/schema/flow/board";
import type {
	IBoardEntryNode,
	IBoardSummary,
} from "../../../lib/schema/flow/board-summary";
import type { IEvent } from "../../../lib/schema/flow/event";
import type { PageListItem } from "../../../state/backend-state/page-state";
import type { ProjectRun } from "../../settings/dashboard/use-project-runs";
import { type IScoreBand, bandOf } from "./flows-overview-tokens";

/** An entry node as the overview lists it: id plus a display name. */
export interface IFlowEntryPoint {
	id: string;
	name: string;
	friendly_name: string;
}

/**
 * One board, plus everything the overview needs to render it. Built from a board *summary*
 * (`getBoardSummaries(appId, ["metrics", "node_types"])`) — every metric here is computed
 * server-side by `scoring.rs` / `Board::summary_metrics`, so the overview never transfers a
 * board's graph.
 */
export interface IFlowRow {
	board: IBoardSummary;
	scores: IBoardScores | undefined;
	worst: number | undefined;
	worstDimension: IScoreCategory | undefined;
	band: IScoreBand;
	coverage: IScoreCoverage;
	causes: IFlaggedPattern[];
	entryPoints: IFlowEntryPoint[];
	connections: number;
	wasm: IBoardWasmUsage;
	variables: IVariableCounts;
	layers: ILayerCounts;
	nodeTotal: number;
	/** Joined from Events on `board_id` — never a property of the board. */
	bindings: IEvent[];
	/** Owned by the board via `page_ids`. */
	pages: PageListItem[];
	versionLabel: string;
}

const entryPointOf = (node: IBoardEntryNode): IFlowEntryPoint => ({
	id: node.nodeId,
	name: node.nodeType,
	friendly_name: node.friendlyName || node.nodeType,
});

/**
 * Events point at boards, never the other way round, so this join is the only
 * way to know what can start a flow. A board with entry points but no row here
 * is unreachable outside the editor.
 */
export function eventsForBoard(events: IEvent[], boardId: string): IEvent[] {
	return events
		.filter((event) => event.board_id === boardId)
		.sort((a, b) => a.name.localeCompare(b.name));
}

export function groupPagesByBoard(
	pages: PageListItem[],
): Map<string, PageListItem[]> {
	const grouped = new Map<string, PageListItem[]>();
	for (const page of pages) {
		if (!page.boardId) continue;
		const bucket = grouped.get(page.boardId);
		if (bucket) bucket.push(page);
		else grouped.set(page.boardId, [page]);
	}
	for (const bucket of grouped.values()) {
		bucket.sort((a, b) => a.name.localeCompare(b.name));
	}
	return grouped;
}

/**
 * A page is only reachable through a route event that names it, so the path
 * comes from `route.eventId -> event.default_page_id`, not from the page.
 */
export function buildRouteByPage(
	events: IEvent[],
	routes: readonly { path: string; eventId: string }[],
): Map<string, string> {
	const pathByEvent = new Map(
		routes.map((route) => [route.eventId, route.path]),
	);
	const routeByPage = new Map<string, string>();
	for (const event of events) {
		const pageId = event.default_page_id;
		if (!pageId || routeByPage.has(pageId)) continue;
		const path = pathByEvent.get(event.id);
		if (path) routeByPage.set(pageId, path);
	}
	return routeByPage;
}

export function buildFlowRow(
	board: IBoardSummary,
	events: IEvent[],
	pagesByBoard: Map<string, PageListItem[]>,
): IFlowRow {
	const scores = board.scores;
	const worst = worstScore(scores);
	const metrics = board.metrics;
	const scoredNodeCount = board.scoredNodeCount ?? 0;
	return {
		board,
		scores,
		worst,
		worstDimension: worstDimension(scores),
		band: bandOf(worst),
		coverage: {
			nodeCount: board.nodeCount,
			scoredNodeCount,
			ratio: board.nodeCount === 0 ? 0 : scoredNodeCount / board.nodeCount,
		},
		causes: (board.flaggedPatterns ?? []).map((pattern) => ({
			node: pattern.node,
			friendlyName: pattern.friendlyName || pattern.node,
			category: pattern.category as IScoreCategory,
			score: pattern.score,
			count: pattern.count,
		})),
		entryPoints: (board.entryNodes ?? []).map(entryPointOf),
		connections: board.connectionCount,
		wasm: {
			packageIds: metrics?.wasmPackages ?? [],
			permissions: (metrics?.wasmPermissions ??
				[]) as IBoardWasmUsage["permissions"],
		},
		variables: {
			total: metrics?.variableCounts.total ?? board.variableCount,
			secret: metrics?.variableCounts.secret ?? 0,
			promptedAtRuntime: metrics?.variableCounts.promptedAtRuntime ?? 0,
		},
		layers: {
			[ILayerType.Collapsed]: metrics?.layerCounts.collapsed ?? 0,
			[ILayerType.Function]: metrics?.layerCounts.function ?? 0,
			[ILayerType.Macro]: metrics?.layerCounts.macro ?? 0,
			[ILayerType.Module]: metrics?.layerCounts.module ?? 0,
			total: metrics?.layerCounts.total ?? board.layerCount,
		} as ILayerCounts,
		nodeTotal: metrics?.totalNodeCount ?? board.nodeCount,
		bindings: eventsForBoard(events, board.id),
		pages: pagesByBoard.get(board.id) ?? [],
		versionLabel: boardVersionLabel(board),
	};
}

export function buildFlowRows(
	boards: IBoardSummary[],
	events: IEvent[],
	pagesByBoard: Map<string, PageListItem[]>,
): IFlowRow[] {
	const unique = Array.from(
		new Map(boards.map((board) => [board.id, board])).values(),
	);
	return unique.map((board) => buildFlowRow(board, events, pagesByBoard));
}

export const BAND_ORDER: IScoreBand[] = [
	"flagged",
	"watch",
	"good",
	"unscored",
];

/** Unscored boards sort last on a dimension sort — they have no value to compare. */
const UNSCORED_SORT_VALUE = 11;

export function sortRows(
	rows: IFlowRow[],
	dimension: IScoreCategory | null,
): IFlowRow[] {
	return [...rows].sort((a, b) => {
		const left = dimension
			? (a.scores?.[dimension] ?? UNSCORED_SORT_VALUE)
			: (a.worst ?? UNSCORED_SORT_VALUE);
		const right = dimension
			? (b.scores?.[dimension] ?? UNSCORED_SORT_VALUE)
			: (b.worst ?? UNSCORED_SORT_VALUE);
		return (
			left - right ||
			b.causes.length - a.causes.length ||
			a.board.name.localeCompare(b.board.name)
		);
	});
}

export function groupIntoBands(rows: IFlowRow[]): Map<IScoreBand, IFlowRow[]> {
	const grouped = new Map<IScoreBand, IFlowRow[]>();
	for (const band of BAND_ORDER) grouped.set(band, []);
	for (const row of rows) grouped.get(row.band)?.push(row);
	return grouped;
}

/** The lowest score any scored board reaches in a dimension. */
export function appWideMinimum(
	rows: IFlowRow[],
	category: IScoreCategory,
): number | undefined {
	const values = rows
		.map((row) => row.scores?.[category])
		.filter((value): value is number => value !== undefined);
	return values.length ? Math.min(...values) : undefined;
}

/** The pair the AI Act conformity component scales on, across the whole app. */
export function appWideSecurityGovernance(
	rows: IFlowRow[],
): number | undefined {
	const security = appWideMinimum(rows, "security");
	const governance = appWideMinimum(rows, "governance");
	if (security === undefined || governance === undefined) return undefined;
	return Math.min(security, governance);
}

export function appWideMinimums(
	rows: IFlowRow[],
): Partial<Record<IScoreCategory, number>> {
	const minimums: Partial<Record<IScoreCategory, number>> = {};
	for (const category of SCORE_CATEGORIES) {
		minimums[category] = appWideMinimum(rows, category);
	}
	return minimums;
}

export type IRunRecency = "minutes" | "hour" | "today" | "older";

export const RUN_RECENCY_ORDER: IRunRecency[] = [
	"minutes",
	"hour",
	"today",
	"older",
];

export const RUN_RECENCY_LABEL: Record<IRunRecency, string> = {
	minutes: "Last few minutes",
	hour: "Past hour",
	today: "Earlier today",
	older: "Older than a day",
};

const MINUTE_MS = 60_000;

export function recencyOf(startedAt: number, now: number): IRunRecency {
	const age = now - startedAt;
	if (age < 5 * MINUTE_MS) return "minutes";
	if (age < 60 * MINUTE_MS) return "hour";
	if (age < 24 * 60 * MINUTE_MS) return "today";
	return "older";
}

export function groupRunsByRecency(
	runs: ProjectRun[],
	now: number,
): Map<IRunRecency, ProjectRun[]> {
	const grouped = new Map<IRunRecency, ProjectRun[]>();
	for (const bucket of RUN_RECENCY_ORDER) grouped.set(bucket, []);
	for (const run of runs) grouped.get(recencyOf(run.startedAt, now))?.push(run);
	return grouped;
}
