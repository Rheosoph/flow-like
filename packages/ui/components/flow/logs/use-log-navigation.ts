import { useCallback, useEffect, useRef, useState } from "react";
import type { ILog } from "../../../lib/schema/flow/log";
import type { ILogMetadata } from "../../../lib/schema/flow/log-metadata";
import type {
	ILogQuery,
	ISummaryLog,
} from "../../../lib/schema/flow/log-query";
import type { IBoardState } from "../../../state/backend-state/board-state";
import { errorsOnly, withTimeRange } from "./filter-model";
import type { IJumpNotice } from "./first-error-strip";
import { levelIndex, logStart } from "./log-format";

const NOTICE_MS = 5_000;
const SAME_TIME_LIMIT = 50;

export interface ILocateTarget {
	start: number;
	node_id?: string | null;
	level: number;
	message: string;
	fingerprint?: string | null;
}

export function targetOf(log: ILog): ILocateTarget {
	return {
		start: logStart(log),
		node_id: log.node_id,
		level: levelIndex(log.log_level),
		message: log.message ?? "",
		fingerprint: log.fingerprint,
	};
}

export function targetOfSummary(log: ISummaryLog): ILocateTarget {
	return {
		start: log.start,
		node_id: log.node_id,
		level: log.log_level,
		message: log.message ?? "",
		fingerprint: log.fingerprint,
	};
}

function matches(row: ILog, target: ILocateTarget): boolean {
	if ((row.node_id ?? null) !== (target.node_id ?? null)) return false;
	if (levelIndex(row.log_level) !== target.level) return false;
	if (row.fingerprint && target.fingerprint) {
		return row.fingerprint === target.fingerprint;
	}
	return (row.message ?? "").startsWith(target.message.slice(0, 200));
}

export interface ILocated {
	index: number;
	log: ILog;
}

/**
 * The row's position under `query`: rows before its timestamp, plus its place
 * among rows logged in the same microsecond. Undefined when the query hides it.
 */
export async function locateRow(
	boardState: IBoardState,
	meta: ILogMetadata,
	query: ILogQuery,
	target: ILocateTarget,
): Promise<ILocated | undefined> {
	const [before, same] = await Promise.all([
		boardState.countRunLogs(
			meta,
			withTimeRange(query, undefined, target.start - 1),
		),
		boardState.queryRunLogs(
			meta,
			withTimeRange(query, target.start, target.start),
			0,
			SAME_TIME_LIMIT,
		),
	]);
	const at = same.findIndex((row) => matches(row, target));
	return at < 0 ? undefined : { index: before + at, log: same[at] };
}

/** The nearest error or fatal row after (or before) `from` under `query`. */
export async function neighbourError(
	boardState: IBoardState,
	meta: ILogMetadata,
	query: ILogQuery,
	from: number | undefined,
	direction: 1 | -1,
): Promise<ILog | undefined> {
	const errors = errorsOnly(query);
	if (direction > 0) {
		const range = withTimeRange(
			errors,
			from === undefined ? undefined : from + 1,
		);
		return (await boardState.queryRunLogs(meta, range, 0, 1))[0];
	}
	const range = withTimeRange(
		errors,
		undefined,
		from === undefined ? undefined : from - 1,
	);
	const count = await boardState.countRunLogs(meta, range);
	if (count <= 0) return undefined;
	return (await boardState.queryRunLogs(meta, range, count - 1, 1))[0];
}

/** First-error jump and E / Shift+E stepping, each resolved to a list index. */
export function useLogNavigation({
	boardState,
	meta,
	query,
	firstError,
	current,
	goTo,
}: {
	boardState: IBoardState;
	meta: ILogMetadata;
	query?: ILogQuery;
	firstError?: ISummaryLog | null;
	current?: ILog;
	goTo: (located: ILocated) => void;
}) {
	const [busy, setBusy] = useState(false);
	const [notice, setNotice] = useState<IJumpNotice>();
	const latest = useRef({ boardState, meta, query, firstError, current, goTo });
	latest.current = { boardState, meta, query, firstError, current, goTo };

	useEffect(() => {
		if (!notice) return;
		const timer = setTimeout(() => setNotice(undefined), NOTICE_MS);
		return () => clearTimeout(timer);
	}, [notice]);

	const run = useCallback(
		async (find: () => Promise<IJumpNotice | ILocated>) => {
			setBusy(true);
			setNotice(undefined);
			try {
				const result = await find();
				if (typeof result === "string") setNotice(result);
				else latest.current.goTo(result);
			} catch (error) {
				console.error("Log navigation failed", error);
				setNotice("failed");
			} finally {
				setBusy(false);
			}
		},
		[],
	);

	const jumpToFirstError = useCallback(
		() =>
			run(async () => {
				const { boardState, meta, query, firstError } = latest.current;
				if (!query || !firstError) return "failed";
				const found = await locateRow(
					boardState,
					meta,
					query,
					targetOfSummary(firstError),
				);
				return found ?? "hidden";
			}),
		[run],
	);

	const stepError = useCallback(
		(direction: 1 | -1) =>
			run(async () => {
				const { boardState, meta, query, current } = latest.current;
				if (!query) return "failed";
				const from = current ? logStart(current) : undefined;
				const next = await neighbourError(
					boardState,
					meta,
					query,
					from,
					direction,
				);
				if (!next) return "no-more-errors";
				return (
					(await locateRow(boardState, meta, query, targetOf(next))) ?? "failed"
				);
			}),
		[run],
	);

	const clearNotice = useCallback(() => setNotice(undefined), []);

	return { busy, notice, jumpToFirstError, stepError, clearNotice };
}
