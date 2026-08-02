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
	connectionCount,
	entryPointNodes,
	flaggedPatterns,
	layerCounts,
	minScores,
	scoreCoverage,
	totalBoardNodeCount,
	variableCounts,
	wasmPackages,
	worstDimension,
	worstScore,
} from "../../../lib/board-metrics";
import type { IBoard, INode } from "../../../lib/schema/flow/board";
import type { IEvent } from "../../../lib/schema/flow/event";
import type { PageListItem } from "../../../state/backend-state/page-state";
import type { ProjectRun } from "../../settings/dashboard/use-project-runs";
import { type IScoreBand, bandOf } from "./flows-overview-tokens";

/** One board, plus everything the overview needs to render it. */
export interface IFlowRow {
	board: IBoard;
	scores: IBoardScores | undefined;
	worst: number | undefined;
	worstDimension: IScoreCategory | undefined;
	band: IScoreBand;
	coverage: IScoreCoverage;
	causes: IFlaggedPattern[];
	entryPoints: INode[];
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
	board: IBoard,
	events: IEvent[],
	pagesByBoard: Map<string, PageListItem[]>,
): IFlowRow {
	const scores = minScores(board);
	const worst = worstScore(scores);
	return {
		board,
		scores,
		worst,
		worstDimension: worstDimension(scores),
		band: bandOf(worst),
		coverage: scoreCoverage(board),
		causes: flaggedPatterns(board),
		entryPoints: entryPointNodes(board),
		connections: connectionCount(board),
		wasm: wasmPackages(board),
		variables: variableCounts(board),
		layers: layerCounts(board),
		nodeTotal: totalBoardNodeCount(board),
		bindings: eventsForBoard(events, board.id),
		pages: pagesByBoard.get(board.id) ?? [],
		versionLabel: boardVersionLabel(board),
	};
}

export function buildFlowRows(
	boards: IBoard[],
	events: IEvent[],
	pagesByBoard: Map<string, PageListItem[]>,
): IFlowRow[] {
	const unique = Array.from(
		new Map(boards.map((board) => [board.id, board])).values(),
	);
	return unique.map((board) => buildFlowRow(board, events, pagesByBoard));
}

export function matchesQuery(row: IFlowRow, query: string): boolean {
	if (!query) return true;
	const haystack = [
		row.board.name,
		row.board.description,
		row.board.stage,
		row.versionLabel,
		...row.pages.map((page) => page.name),
		...row.bindings.map((event) => event.name),
	]
		.join(" ")
		.toLowerCase();
	return haystack.includes(query);
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
