"use client";

import { useQuery } from "@tanstack/react-query";
import type { FlowVersionState } from "../lib/device-management/hub/endpoints";
import { useBackend } from "../state/backend-state";

/** Compare the saved draft with published snapshots, independently of the selected pin. */
export function useBoardVersionCurrent(
	appId: string,
	boardId: string,
	enabled = true,
) {
	const backend = useBackend();
	return useQuery<FlowVersionState | null>({
		queryKey: ["getBoardVersionCurrent", appId, boardId],
		queryFn: async () =>
			backend.boardState.getBoardVersionCurrent?.(appId, boardId) ?? null,
		enabled: enabled && Boolean(appId && boardId),
		staleTime: 0,
		retry: false,
	});
}
