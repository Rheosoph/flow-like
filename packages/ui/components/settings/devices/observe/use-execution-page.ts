"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import type { AgentRead } from "../../../../lib/device-management/agent-reads";

export interface ExecutionPage<T> {
	rows: T[];
	next: number | null;
	limitReached?: boolean;
}

/** Keeps decrypted pages in this mounted view only; late responses cannot restore a closed view. */
export function useExecutionPage<T>(
	read: (offset: number) => Promise<AgentRead<ExecutionPage<T>>>,
	identity?: (row: T) => string,
) {
	const generation = useRef(0);
	const [state, setState] = useState<
		ExecutionPage<T> & {
			busy: boolean;
			error: string | null;
			unsupported: boolean;
		}
	>({ rows: [], next: null, busy: true, error: null, unsupported: false });

	const load = useCallback(
		(offset: number) => {
			const request = ++generation.current;
			setState((previous) => ({
				...previous,
				...(offset === 0 ? { rows: [], next: null, limitReached: false } : {}),
				busy: true,
				error: null,
			}));
			void read(offset).then(
				(answer) => {
					if (request !== generation.current) return;
					if (answer.kind === "unsupported") {
						setState({
							rows: [],
							next: null,
							busy: false,
							error: null,
							unsupported: true,
						});
						return;
					}
					setState((previous) => {
						const rows =
							offset === 0
								? answer.data.rows
								: [...previous.rows, ...answer.data.rows];
						return {
							// A newly finished run can shift offset pages while this view is open.
							rows: identity
								? [...new Map(rows.map((row) => [identity(row), row])).values()]
								: rows,
							next: answer.data.next,
							limitReached: answer.data.limitReached,
							busy: false,
							error: null,
							unsupported: false,
						};
					});
				},
				(error: unknown) => {
					if (request !== generation.current) return;
					setState({
						rows: [],
						next: null,
						busy: false,
						unsupported: false,
						error: error instanceof Error ? error.message : String(error),
					});
				},
			);
		},
		[read, identity],
	);

	useEffect(() => {
		load(0);
		return () => {
			generation.current++;
		};
	}, [load]);

	return {
		...state,
		refresh: () => load(0),
		loadMore: () => {
			if (!state.busy && state.next !== null) load(state.next);
		},
	};
}
