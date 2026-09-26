import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { useAppPermissions } from "../../../hooks/use-app-permissions";
import { RolePermissions } from "../../../lib/permission/role-permission";
import type { ILog } from "../../../lib/schema/flow/log";
import type { ILogMetadata } from "../../../lib/schema/flow/log-metadata";
import type { ILogQuery } from "../../../lib/schema/flow/log-query";
import type { IBoardState } from "../../../state/backend-state/board-state";
import { useLogAggregation } from "../../../state/log-aggregation-state";
import { queryKey } from "./filter-model";
import { LogPageCache } from "./log-page-cache";

/** Changes when the run gains logs, so a live run never serves stale pages. */
export function runKey(meta: ILogMetadata): string {
	return [
		meta.app_id,
		meta.board_id,
		meta.run_id,
		meta.end,
		meta.logs ?? "",
	].join("/");
}

export function useRunLogSummary(
	boardState: IBoardState,
	meta: ILogMetadata | undefined,
) {
	return useQuery({
		queryKey: ["run-log-summary", meta ? runKey(meta) : ""],
		queryFn: () => (meta ? boardState.getRunLogSummary(meta) : null),
		enabled: !!meta,
		retry: 1,
		staleTime: 60_000,
	});
}

/** Loads the current run's summary for the node badges, whether or not the Logs panel is open. */
export function useSyncCurrentRunSummary(boardState: IBoardState) {
	const meta = useLogAggregation((state) => state.currentMetadata);
	const setCurrentSummary = useLogAggregation(
		(state) => state.setCurrentSummary,
	);
	const permissions = useAppPermissions(
		meta?.is_remote ? meta.app_id : undefined,
	);
	const readable =
		!meta?.is_remote || permissions.canStrict(RolePermissions.ReadLogs);
	const { data } = useRunLogSummary(boardState, readable ? meta : undefined);
	const runId = meta?.run_id;
	useEffect(() => {
		if (runId && data !== undefined) setCurrentSummary(runId, data);
	}, [data, runId, setCurrentSummary]);
}

export function useRunLogCount(
	boardState: IBoardState,
	meta: ILogMetadata,
	query: ILogQuery,
	enabled = true,
) {
	return useQuery({
		queryKey: ["run-log-count", runKey(meta), queryKey(query)],
		queryFn: () => boardState.countRunLogs(meta, query),
		placeholderData: keepPreviousData,
		retry: 1,
		staleTime: 60_000,
		enabled,
	});
}

interface ISettledList {
	key: string;
	query: ILogQuery;
	count: number;
}

export interface ILogWindow {
	/** Rows of the query on screen, which lags a new query until its count arrives. */
	count: number;
	query?: ILogQuery;
	/** Identifies the run and query on screen. */
	listKey: string;
	/** Changes whenever cached pages are dropped. */
	cacheKey: string;
	/** Bumps when a page lands. */
	version: number;
	loading: boolean;
	error?: Error;
	/** A page under the viewport failed to load. */
	pageFailed: boolean;
	row(index: number): ILog | undefined;
	failed(index: number): boolean;
	ensure(first: number, last: number): void;
	fetchRange(offset: number, limit: number): Promise<ILog[]>;
	retry(): void;
}

/**
 * Windowed access to one run's filtered logs: the count sizes the scrollbar,
 * and only the pages under the viewport are fetched. The previous query stays
 * on screen until the next one's count is known, so filtering never flashes
 * an empty list.
 */
export function useLogWindow(
	boardState: IBoardState,
	meta: ILogMetadata,
	query: ILogQuery,
): ILogWindow {
	const countQuery = useRunLogCount(boardState, meta, query);
	const key = `${runKey(meta)}|${queryKey(query)}`;
	const live = countQuery.isPlaceholderData ? undefined : countQuery.data;

	const [settled, setSettled] = useState<ISettledList>();
	if (live !== undefined && (settled?.key !== key || settled.count !== live)) {
		setSettled({ key, query, count: live });
	}

	const [retries, setRetries] = useState(0);
	const cacheKey = settled ? `${settled.key}#${settled.count}~${retries}` : "";
	const cacheRef = useRef<{ key: string; cache: LogPageCache<ILog> }>(
		undefined,
	);
	if (cacheRef.current?.key !== cacheKey) {
		cacheRef.current = { key: cacheKey, cache: new LogPageCache<ILog>() };
	}
	const cache = cacheRef.current.cache;
	const [version, setVersion] = useState(0);

	const latest = useRef({ boardState, meta, settled });
	latest.current = { boardState, meta, settled };

	const fetchRange = useCallback(
		(offset: number, limit: number): Promise<ILog[]> => {
			const { boardState, meta, settled } = latest.current;
			if (!settled) return Promise.resolve([]);
			return boardState.queryRunLogs(meta, settled.query, offset, limit);
		},
		[],
	);

	const ensure = useCallback(
		(first: number, last: number) => {
			const entry = cacheRef.current;
			const current = latest.current.settled;
			if (!entry || !current) return;
			const settle = () => {
				if (cacheRef.current === entry) setVersion((v) => v + 1);
			};
			for (const page of entry.cache.pagesFor(first, last, current.count)) {
				entry.cache.touch(page);
				if (entry.cache.hasFailed(page)) continue;
				entry.cache.request(page, fetchRange)?.then(settle, settle);
			}
		},
		[fetchRange],
	);

	const row = useCallback((index: number) => cache.row(index), [cache]);
	const failed = useCallback(
		(index: number) => cache.hasFailed(cache.pageOf(index)),
		[cache],
	);
	const retry = useCallback(() => {
		setRetries((r) => r + 1);
		void countQuery.refetch();
	}, [countQuery.refetch]);

	return {
		count: settled?.count ?? 0,
		query: settled?.query,
		listKey: settled?.key ?? "",
		cacheKey,
		version,
		loading: !settled && countQuery.isPending,
		error: countQuery.error ?? undefined,
		pageFailed: cache.hasFailures,
		row,
		failed,
		ensure,
		fetchRange,
		retry,
	};
}
