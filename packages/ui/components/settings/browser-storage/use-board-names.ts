"use client";

import { useQueries } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import { useBackend } from "../../../state/backend-state";

export interface BoardNames {
	/** False until the app list arrived, so "app unavailable" is never claimed early. */
	readonly appsLoaded: boolean;
	readonly appName: (appId: string | undefined) => string | undefined;
	readonly boardName: (
		appId: string | undefined,
		boardId: string,
	) => string | undefined;
	readonly hasApp: (appId: string | undefined) => boolean;
}

const summaryData = <T>(results: readonly { data?: T }[]) =>
	results.map((result) => result.data);

/** Names come from board summaries (a cached listing), never from loading boards. */
export function useBoardNames(appIds: readonly string[]): BoardNames {
	const backend = useBackend();
	const apps = useInvoke(backend.appState.getApps, backend.appState, []);
	const appNames = useMemo(
		() =>
			new Map(
				(apps.data ?? []).map(([app, metadata]) => [
					app.id,
					metadata?.name || app.id,
				]),
			),
		[apps.data],
	);
	const listedAppIds = useMemo(
		() => [...new Set(appIds)].filter((appId) => appNames.has(appId)).sort(),
		[appIds, appNames],
	);
	const summaries = useQueries({
		queries: listedAppIds.map((appId) => ({
			queryKey: ["browser-storage", "board-names", appId],
			queryFn: () => backend.boardState.getBoardSummaries(appId),
			staleTime: 5 * 60_000,
			meta: { persist: false },
		})),
		combine: summaryData,
	});
	const boardNames = useMemo(() => {
		const names = new Map<string, string>();
		summaries.forEach((boards, index) => {
			for (const board of boards ?? []) {
				names.set(`${listedAppIds[index]}\u001f${board.id}`, board.name);
			}
		});
		return names;
	}, [listedAppIds, summaries]);

	const appName = useCallback(
		(appId: string | undefined) => (appId ? appNames.get(appId) : undefined),
		[appNames],
	);
	const boardName = useCallback(
		(appId: string | undefined, boardId: string) =>
			boardNames.get(`${appId ?? ""}\u001f${boardId}`),
		[boardNames],
	);
	const hasApp = useCallback(
		(appId: string | undefined) => !!appId && appNames.has(appId),
		[appNames],
	);
	return { appsLoaded: apps.isSuccess, appName, boardName, hasApp };
}
