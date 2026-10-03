"use client";

import { useCallback, useState } from "react";
import { RECORD_BUFFER } from "../../../../lib/device-management/workspace/streams";

export interface OlderRecords {
	/** Older records may exist beyond what is on screen. */
	available: boolean;
	loading: boolean;
	load(): void;
}

/**
 * A stream keeps the newest records and drops older pages from memory, so
 * "Load older" is offered once the buffer is full, until a try brings nothing
 * back (the device has no older record, or deleted them).
 */
export function useOlderRecords(
	count: number,
	loadOlder: () => Promise<void>,
): OlderRecords {
	const [loading, setLoading] = useState(false);
	const [triedAt, setTriedAt] = useState<number | null>(null);
	const load = useCallback(() => {
		setLoading(true);
		void loadOlder()
			.catch(() => undefined)
			.finally(() => {
				setLoading(false);
				setTriedAt(count);
			});
	}, [loadOlder, count]);
	return {
		available: count >= RECORD_BUFFER && triedAt !== count,
		loading,
		load,
	};
}
