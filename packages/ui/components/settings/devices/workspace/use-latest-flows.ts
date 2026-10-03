"use client";

import { useQueries } from "@tanstack/react-query";
import { useCallback, useMemo } from "react";
import {
	type LatestFlows,
	desktopFlowVersionCommands,
	desktopLatestFlows,
	hubLatestFlows,
} from "../../../../lib/device-management/latest-flows";
import type {
	AppMode,
	EventFlowState,
} from "../../../../lib/device-management/model/app-plan";
import type { IApiState } from "../../../../state/backend-state/api-state";
import type { IProfile } from "../../../../types";
import { useDeviceAuth, useDeviceWorkspace } from "./device-workspace-provider";

/* Events that follow Latest: where their flow is read and published as a version. */

/** Test seam: the desktop commands are only reachable inside the desktop app. */
export const latestFlowSeams = { commands: desktopFlowVersionCommands };

export interface LatestFlowsSource {
	api: IApiState;
	profile: IProfile;
	desktop: boolean;
	/** The signed-in account a local-only app's flow version is published by. */
	account?: string;
}

/**
 * Where an app's flows are read and published as versions: the hub for an
 * online app, this computer for a local-only one. `null` where nothing can
 * say: a local-only app outside the desktop app, or no app yet.
 */
export function latestFlowsOf(
	mode: AppMode | null | undefined,
	source: LatestFlowsSource,
): LatestFlows | null {
	if (mode === "online") return hubLatestFlows(source.api, source.profile);
	if (mode === "offline" && source.desktop)
		return desktopLatestFlows(source.account || undefined, () =>
			latestFlowSeams.commands(),
		);
	return null;
}

export function useLatestFlows(
	mode: AppMode | null | undefined,
): LatestFlows | null {
	const { hub, deps } = useDeviceWorkspace();
	const { account } = useDeviceAuth();
	const { api, profile } = hub;
	const desktop = deps.platform === "desktop";
	return useMemo(
		() => latestFlowsOf(mode, { api, profile, desktop, account }),
		[mode, api, profile, desktop, account],
	);
}

const FLOW_STALE_MS = 30_000;

export const flowVersionKey = (
	scopeKey: string,
	appId: string,
	boardId: string,
) => ["devices", scopeKey, "flow-version", appId, boardId] as const;

/**
 * Each flow as a version, by flow id. A flow is absent while it is loading and
 * when its read failed: unknown, never "no edits". `boardIds` must keep its
 * identity while its content is the same.
 */
export function useFlowStates(
	appId: string | undefined,
	mode: AppMode | null | undefined,
	boardIds: readonly string[],
): Readonly<Record<string, EventFlowState>> {
	const { scopeKey } = useDeviceWorkspace();
	const flows = useLatestFlows(mode);
	const combine = useCallback(
		(results: { data: EventFlowState | undefined }[]) => {
			const states: Record<string, EventFlowState> = {};
			for (const [index, boardId] of boardIds.entries()) {
				const data = results[index]?.data;
				if (data !== undefined) states[boardId] = data;
			}
			return states;
		},
		[boardIds],
	);
	return useQueries({
		queries: boardIds.map((boardId) => ({
			queryKey: flowVersionKey(scopeKey, appId ?? "", boardId),
			queryFn: async (): Promise<EventFlowState> => {
				if (!flows || !appId) throw new Error("No app to read the flow of.");
				const read = await flows.read(appId, boardId);
				return read.kind === "ok" ? read.data : "missing_on_hub";
			},
			enabled: !!flows && !!appId,
			staleTime: FLOW_STALE_MS,
			retry: false,
		})),
		combine,
	});
}
