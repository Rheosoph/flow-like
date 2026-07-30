"use client";

import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";
import type { IBoard, ILogMetadata } from "../../../lib";
import { useBackend } from "../../../state/backend-state";

/** Runs carry microsecond epoch timestamps (see `LogMeta` in the core crate). */
const MICROS_PER_MS = 1_000;
const WINDOW_MS = 24 * 60 * 60 * 1000;
const RUNS_PER_BOARD = 200;
const TREND_BUCKETS = 12;
/**
 * Health is aggregated with one run-log query per board, so the fan-out is
 * bounded. Real projects sit far below this; the cap only stops a pathological
 * app from firing dozens of queries a minute.
 */
const MAX_BOARDS_SAMPLED = 25;

export interface ProjectRun {
	runId: string;
	boardId: string;
	boardName: string;
	eventId: string;
	nodeId: string;
	/** Epoch milliseconds. */
	startedAt: number;
	/** Microseconds — the unit `formatDuration` expects. */
	durationMicros: number;
	failed: boolean;
	warned: boolean;
}

export interface BoardRunHealth {
	boardId: string;
	total: number;
	failed: number;
}

export interface SurfaceRunHealth {
	total: number;
	failed: number;
	/** Epoch milliseconds of the newest run for this surface. */
	lastAt: number | null;
}

export interface ProjectRunHealth {
	/** True once the aggregation has resolved at least once. */
	ready: boolean;
	isLoading: boolean;
	/** Runs started inside the trailing 24h window. */
	windowRuns: number;
	windowFailed: number;
	/** null when nothing ran in the window — never render a fake 100%. */
	successRate: number | null;
	/** Microseconds, null when the window is empty. */
	p95Micros: number | null;
	/** Newest run overall, independent of the window. */
	lastRunAt: number | null;
	/** Any run ever observed, used to decide the dashboard mode. */
	hasEverRun: boolean;
	hasEverSucceeded: boolean;
	/** Newest first, capped for display. */
	recent: ProjectRun[];
	byBoard: Map<string, BoardRunHealth>;
	/** Keyed by event id — drives the Surfaces table. */
	byEvent: Map<string, SurfaceRunHealth>;
	/** Runs bucketed into 12 two-hour slots across the window, oldest first. */
	trend: number[];
}

function percentile(sorted: number[], p: number): number | null {
	if (sorted.length === 0) return null;
	const index = Math.min(
		sorted.length - 1,
		Math.max(0, Math.ceil((p / 100) * sorted.length) - 1),
	);
	return sorted[index];
}

/**
 * Aggregate recent runs across every board of an app into a single health
 * summary.
 *
 * This reads the run log (`listRuns`) rather than the analytics API on
 * purpose: analytics is server-backed and unavailable for offline projects,
 * whereas the run log exists wherever the app has actually executed. Every
 * number here is derived from real runs — when there are none the fields go
 * null so callers render an empty state instead of a fabricated zero.
 */
export function useProjectRuns(
	appId: string | undefined,
	boards: IBoard[] | undefined,
): ProjectRunHealth {
	const backend = useBackend();
	const boardIds = useMemo(
		() =>
			(boards ?? [])
				.map((board) => board.id)
				.sort()
				.slice(0, MAX_BOARDS_SAMPLED),
		[boards],
	);
	const boardNames = useMemo(() => {
		const map = new Map<string, string>();
		for (const board of boards ?? []) map.set(board.id, board.name);
		return map;
	}, [boards]);

	const query = useQuery<ILogMetadata[]>({
		queryKey: ["project-runs", appId, boardIds],
		enabled: !!appId && boardIds.length > 0,
		staleTime: 30_000,
		refetchInterval: 60_000,
		queryFn: async () => {
			if (!appId) return [];
			const perBoard = await Promise.all(
				boardIds.map(async (boardId) => {
					try {
						return await backend.boardState.listRuns(
							appId,
							boardId,
							undefined,
							undefined,
							undefined,
							undefined,
							undefined,
							0,
							RUNS_PER_BOARD,
						);
					} catch {
						// A board with no run store yet simply has no runs.
						return [] as ILogMetadata[];
					}
				}),
			);
			return perBoard.flat();
		},
	});

	return useMemo(
		() => ({
			ready: query.isFetched,
			isLoading: query.isLoading,
			...summarize(toRuns(query.data ?? [], boardNames)),
		}),
		[query.data, query.isFetched, query.isLoading, boardNames],
	);
}

/**
 * Decode raw run metadata. Exported because the unit semantics here are a
 * boundary worth testing: `start`/`end` are microsecond epoch values and
 * `log_level` is the numeric protobuf enum (3 = Error, 4 = Fatal).
 */
export function toRuns(
	metas: ILogMetadata[],
	boardNames: Map<string, string>,
): ProjectRun[] {
	return metas
		.map((meta) => {
			const level = Number(meta.log_level ?? 0);
			return {
				runId: meta.run_id,
				boardId: meta.board_id,
				boardName: boardNames.get(meta.board_id) ?? "Deleted flow",
				eventId: meta.event_id,
				nodeId: meta.node_id,
				startedAt: Math.floor(meta.start / MICROS_PER_MS),
				durationMicros: Math.max(0, meta.end - meta.start),
				failed: level >= 3,
				warned: level === 2,
			};
		})
		.sort((a, b) => b.startedAt - a.startedAt);
}

function bucketize(runs: ProjectRun[], cutoff: number): number[] {
	const buckets = new Array<number>(TREND_BUCKETS).fill(0);
	const span = WINDOW_MS / TREND_BUCKETS;
	for (const run of runs) {
		const slot = Math.min(
			TREND_BUCKETS - 1,
			Math.max(0, Math.floor((run.startedAt - cutoff) / span)),
		);
		buckets[slot] += 1;
	}
	return buckets;
}

function groupRuns(runs: ProjectRun[]): {
	byBoard: Map<string, BoardRunHealth>;
	byEvent: Map<string, SurfaceRunHealth>;
} {
	const byBoard = new Map<string, BoardRunHealth>();
	const byEvent = new Map<string, SurfaceRunHealth>();

	for (const run of runs) {
		const board = byBoard.get(run.boardId) ?? {
			boardId: run.boardId,
			total: 0,
			failed: 0,
		};
		board.total += 1;
		if (run.failed) board.failed += 1;
		byBoard.set(run.boardId, board);

		if (!run.eventId) continue;
		const surface = byEvent.get(run.eventId) ?? {
			total: 0,
			failed: 0,
			lastAt: null,
		};
		surface.total += 1;
		if (run.failed) surface.failed += 1;
		surface.lastAt = Math.max(surface.lastAt ?? 0, run.startedAt);
		byEvent.set(run.eventId, surface);
	}

	return { byBoard, byEvent };
}

export function summarize(
	runs: ProjectRun[],
): Omit<ProjectRunHealth, "ready" | "isLoading"> {
	const cutoff = Date.now() - WINDOW_MS;
	const windowed = runs.filter((run) => run.startedAt >= cutoff);
	const windowFailed = windowed.filter((run) => run.failed).length;
	const durations = windowed
		.map((run) => run.durationMicros)
		.sort((a, b) => a - b);
	const { byBoard, byEvent } = groupRuns(windowed);

	return {
		windowRuns: windowed.length,
		windowFailed,
		successRate:
			windowed.length > 0
				? ((windowed.length - windowFailed) / windowed.length) * 100
				: null,
		p95Micros: percentile(durations, 95),
		lastRunAt: runs[0]?.startedAt ?? null,
		hasEverRun: runs.length > 0,
		hasEverSucceeded: runs.some((run) => !run.failed),
		recent: runs.slice(0, 8),
		byBoard,
		byEvent,
		trend: bucketize(windowed, cutoff),
	};
}
