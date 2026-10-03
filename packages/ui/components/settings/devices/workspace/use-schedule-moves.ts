"use client";

import { useMemo } from "react";
import {
	type HubResult,
	type ScheduleGiveBack,
	type ScheduleRefusal,
	type ScheduleRelease,
	giveBackSchedule,
	releaseSchedule,
} from "../../../../lib/device-management/hub/endpoints";
import { deviceKeys } from "../../../../lib/device-management/hub/queries";
import type { DeviceWorkspace } from "../../../../lib/device-management/workspace/types";
import { useDeviceWorkspace } from "./device-workspace-provider";

/*
 * Moving a schedule between the hub and a device service. Both moves need the
 * right to edit the app's events; the hub decides, and every move is followed
 * by a new read of where the app's schedules run.
 */

export type ScheduleMoveResult<T> = HubResult<T> | ScheduleRefusal;

export interface ScheduleMoves {
	/** Lets one service take the schedule off the hub once it runs. */
	release(
		eventId: string,
		deviceId: string,
		serviceId: string,
	): Promise<ScheduleMoveResult<ScheduleRelease>>;
	/** Hands the schedule back to the hub, whatever state the device is in. */
	giveBack(eventId: string): Promise<ScheduleMoveResult<ScheduleGiveBack>>;
}

export function scheduleMoves(
	workspace: Pick<DeviceWorkspace, "hub" | "deps" | "scopeKey">,
	appId: string,
): ScheduleMoves {
	const { api, profile } = workspace.hub;
	const reread = async <T>(result: T): Promise<T> => {
		await workspace.deps.queryClient.invalidateQueries({
			queryKey: deviceKeys.appPlacements(workspace.scopeKey, appId),
		});
		return result;
	};
	return {
		release: async (eventId, deviceId, serviceId) =>
			reread(
				await releaseSchedule(
					api,
					profile,
					appId,
					eventId,
					deviceId,
					serviceId,
				),
			),
		giveBack: async (eventId) =>
			reread(await giveBackSchedule(api, profile, appId, eventId)),
	};
}

export function useScheduleMoves(appId: string): ScheduleMoves {
	const workspace = useDeviceWorkspace();
	return useMemo(() => scheduleMoves(workspace, appId), [workspace, appId]);
}
