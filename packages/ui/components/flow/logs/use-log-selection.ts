import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ILog } from "../../../lib/schema/flow/log";
import { logKey, logStart } from "./log-format";
import type { ILogWindow } from "./use-log-window";

const RANGE_LIMIT = 5_000;

async function rowsBetween(
	list: ILogWindow,
	from: number,
	to: number,
): Promise<ILog[]> {
	const cached: ILog[] = [];
	for (let i = from; i <= to; i++) {
		const row = list.row(i);
		if (row) cached.push(row);
	}
	if (cached.length === to - from + 1) return cached;
	try {
		return await list.fetchRange(from, Math.min(to - from + 1, RANGE_LIMIT));
	} catch {
		return cached;
	}
}

/**
 * Selected rows are kept as snapshots, so copying never depends on a row
 * still being loaded or even matching the current filters.
 */
export function useLogSelection(list: ILogWindow) {
	const [selected, setSelected] = useState<ReadonlyMap<string, ILog>>(
		() => new Map(),
	);
	const anchor = useRef<number | undefined>(undefined);
	const listRef = useRef(list);
	listRef.current = list;

	useEffect(() => {
		if (list.listKey) anchor.current = undefined;
	}, [list.listKey]);

	const toggle = useCallback((index: number, log: ILog, shift: boolean) => {
		const from = anchor.current;
		if (shift && from !== undefined && from !== index) {
			const [lo, hi] = from < index ? [from, index] : [index, from];
			void rowsBetween(listRef.current, lo, hi).then((rows) => {
				setSelected((prev) => {
					const next = new Map(prev);
					for (const row of rows) next.set(logKey(row), row);
					return next;
				});
			});
			anchor.current = index;
			return;
		}
		anchor.current = index;
		setSelected((prev) => {
			const next = new Map(prev);
			const key = logKey(log);
			if (next.has(key)) next.delete(key);
			else next.set(key, log);
			return next;
		});
	}, []);

	const clear = useCallback(() => {
		anchor.current = undefined;
		setSelected(new Map());
	}, []);

	const keys = useMemo(() => new Set(selected.keys()), [selected]);
	const logs = useMemo(
		() => [...selected.values()].sort((a, b) => logStart(a) - logStart(b)),
		[selected],
	);

	return { keys, logs, count: selected.size, toggle, clear };
}
