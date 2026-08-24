import type { IBoardScores } from "../../board-metrics";
import type {
	IExecutionMode,
	IExecutionStage,
	ILogLevel,
	ISystemTime,
	IVariable,
} from "./board";

/**
 * A board without its graph — what `GET /apps/{app}/board/summaries` returns. Served from a
 * database cache, so listing an app's boards costs no object-storage reads and no full-board
 * transfer. Prefer this over `getBoards` wherever the nodes are not needed.
 */
export interface IBoardSummary {
	id: string;
	name: string;
	description: string;
	stage: IExecutionStage;
	executionMode: IExecutionMode;
	logLevel: ILogLevel;
	version: [number, number, number];
	nodeCount: number;
	connectionCount: number;
	variableCount: number;
	layerCount: number;
	commentCount: number;
	scores?: IBoardScores;
	pages: IBoardSummaryPage[];
	/** Only present when requested with `include: ["node_types"]`. */
	nodeTypes?: string[];
	/** Only present when requested with `include: ["node_types"]`. */
	entryNodes?: IBoardEntryNode[];
	/** Absent only for summaries cached before it was recorded. */
	updatedAt?: ISystemTime | null;
	/** Only present when requested with `include: ["metrics"]`. */
	scoredNodeCount?: number;
	/** Only present when requested with `include: ["metrics"]`. */
	flaggedPatterns?: IBoardFlaggedPattern[];
	/** Only present when requested with `include: ["metrics"]`. */
	metrics?: IBoardSummaryMetrics;
}

/** Server-side twin of `flaggedPatterns()` in `board-metrics.ts` (`scoring.rs::FlaggedPattern`). */
export interface IBoardFlaggedPattern {
	node: string;
	/** Empty on rows persisted before it was recorded; fall back to `node`. */
	friendlyName: string;
	category: string;
	score: number;
	count: number;
}

export interface IBoardSummaryMetrics {
	totalNodeCount: number;
	wasmPackages: string[];
	wasmPermissions: string[];
	variableCounts: { total: number; secret: number; promptedAtRuntime: number };
	layerCounts: {
		total: number;
		collapsed: number;
		function: number;
		macro: number;
		module: number;
	};
}

export interface IBoardSummaryPage {
	appId: string;
	pageId: string;
	boardId?: string | null;
	name: string;
	description?: string | null;
	updatedAt?: string | null;
}

export interface IBoardEntryNode {
	nodeId: string;
	nodeType: string;
	/** Empty on rows persisted before it was recorded; fall back to `nodeType`. */
	friendlyName?: string;
}

export type IBoardSummaryInclude = "node_types" | "metrics";

/**
 * The little a listing UI needs to know about a board. Satisfied by a summary directly and by
 * a full board through [`boardListing`], so components can stop depending on `IBoard` without
 * forcing every caller to switch data source at once.
 */
export interface IBoardListing {
	id: string;
	name: string;
	description: string;
	nodeCount: number;
	updatedAt?: ISystemTime | null;
}

export function boardListing(
	board:
		| IBoardSummary
		| {
				id: string;
				name: string;
				description: string;
				nodes: object;
				updated_at?: ISystemTime | null;
		  },
): IBoardListing {
	if ("nodeCount" in board) {
		return {
			id: board.id,
			name: board.name,
			description: board.description,
			nodeCount: board.nodeCount,
			updatedAt: board.updatedAt ?? null,
		};
	}
	return {
		id: board.id,
		name: board.name,
		description: board.description,
		nodeCount: Object.keys(board.nodes).length,
		updatedAt: board.updated_at ?? null,
	};
}

/** `GET /apps/{app}/board/variables`: every board's variables, secret values stripped. */
export interface IBoardVariables {
	board_id: string;
	board_name: string;
	variables: Record<string, IVariable>;
	/** Only the schema refs the variables reach, enough to resolve struct variables. */
	refs: Record<string, string>;
}
