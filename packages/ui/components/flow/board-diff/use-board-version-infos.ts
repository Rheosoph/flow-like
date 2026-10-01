"use client";

import { useMemo } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import type { IBoardVersionInfo } from "../../../lib/schema/flow/board-version";
import { useBackend } from "../../../state/backend-state";

async function getBoardVersionInfos(): Promise<IBoardVersionInfo[]> {
	return [];
}

/**
 * Published versions with who and when. Goes through `useInvoke` so the desktop's
 * background server refresh (injected under the method's own key) reaches the dialog;
 * hosts without the endpoint fall back to bare version numbers.
 */
export function useBoardVersionInfos(
	appId: string,
	boardId: string,
	enabled: boolean,
) {
	const state = useBackend().boardState;
	const supported = typeof state.getBoardVersionInfos === "function";
	const ready = enabled && Boolean(appId && boardId);
	const infos = useInvoke(
		state.getBoardVersionInfos ?? getBoardVersionInfos,
		state,
		[appId, boardId],
		ready && supported,
	);
	const versions = useInvoke(
		state.getBoardVersions,
		state,
		[appId, boardId],
		ready && !supported,
	);

	const data = useMemo<IBoardVersionInfo[] | undefined>(() => {
		if (supported) return infos.data;
		return versions.data?.map((version) => ({ version }));
	}, [infos.data, supported, versions.data]);

	return {
		data,
		isLoading: supported ? infos.isLoading : versions.isLoading,
	};
}
