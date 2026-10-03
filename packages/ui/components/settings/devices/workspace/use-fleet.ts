"use client";

import { useEffect, useMemo } from "react";
import {
	buildDeviceView,
	buildServiceViews,
} from "../../../../lib/device-management/model/attention";
import {
	type Coverage,
	coverage,
} from "../../../../lib/device-management/model/coverage";
import type {
	AgeState,
	CopyRef,
	DeviceViewModel,
	FreshnessReason,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import type {
	DeviceWorkspace,
	FleetDeviceState,
} from "../../../../lib/device-management/workspace/types";
import { useAttentionState } from "./use-attention";

/** A visible view counts as use for the idle lock (M-DATA §3.2.3); re-stated once a minute. */
export const VIEW_TOUCH_MS = 60_000;

const pageVisible = () => globalThis.document?.visibilityState !== "hidden";

/** While mounted: encrypted status is polled for the device, and its keys count as in use while the page is visible. */
function watchDevice(workspace: DeviceWorkspace, deviceId: string) {
	const release = workspace.fleet.watch(deviceId);
	const touch = () => {
		if (pageVisible()) workspace.touch(deviceId);
	};
	touch();
	const timer = setInterval(touch, VIEW_TOUCH_MS);
	globalThis.document?.addEventListener("visibilitychange", touch);
	return () => {
		release();
		clearInterval(timer);
		globalThis.document?.removeEventListener("visibilitychange", touch);
	};
}

/**
 * One device as the screens show it: hub row, presence, keys, live state,
 * services (or why they can't be read), health and its attention items.
 * `undefined` while the device is not in the hub list.
 */
export function useDeviceView(
	deviceId: string | undefined,
	options: { watch?: boolean } = {},
): DeviceViewModel | undefined {
	const { workspace, input, items } = useAttentionState();
	const watch = options.watch !== false;
	useEffect(() => {
		if (!deviceId || !watch) return;
		return watchDevice(workspace, deviceId);
	}, [workspace, deviceId, watch]);
	return useMemo(
		() => (deviceId ? buildDeviceView(deviceId, input, items) : undefined),
		[deviceId, input, items],
	);
}

/**
 * Every device of the hub list, in list order (N1, rail, app views). `watch`
 * polls encrypted status for each device with keys here; locked devices cost nothing.
 */
export function useDeviceViews(
	options: { watch?: boolean } = {},
): DeviceViewModel[] {
	const { workspace, input, items } = useAttentionState();
	const watch = options.watch === true;
	const withKeys = useMemo(
		() =>
			input.local.vaults
				.map((vault) => vault.deviceId)
				.sort()
				.join("|"),
		[input.local.vaults],
	);
	useEffect(() => {
		if (!watch || !withKeys) return;
		const releases = withKeys
			.split("|")
			.map((deviceId) => workspace.fleet.watch(deviceId));
		return () => {
			for (const release of releases) release();
		};
	}, [workspace, watch, withKeys]);
	return useMemo(
		() =>
			input.devices.flatMap((row) => {
				const view = buildDeviceView(row.device_id, input, items);
				return view ? [view] : [];
			}),
		[input, items],
	);
}

export interface ServiceViewRead {
	service: ServiceView | undefined;
	/** Why no row is known while the device's services can't be read (R6: never "empty"). */
	unavailable?: { state: AgeState; reason?: CopyRef<FreshnessReason> };
	device: DeviceViewModel | undefined;
}

export function useServiceView(
	deviceId: string | undefined,
	serviceId: string | undefined,
): ServiceViewRead {
	const device = useDeviceView(deviceId);
	const { input } = useAttentionState();
	return useMemo(() => {
		if (!deviceId) return { service: undefined, device };
		const services = buildServiceViews(deviceId, input);
		if (!Array.isArray(services))
			return { service: undefined, unavailable: services, device };
		return {
			service: services.find((row) => row.serviceId === serviceId),
			device,
		};
	}, [deviceId, serviceId, input, device]);
}

/** "Status from 3 of 5 devices · 2 locked" (N1, App › Devices); `appId` narrows it to one app. */
export function useCoverage(appId?: string): Coverage {
	const { input } = useAttentionState();
	return useMemo(() => coverage(input, appId), [input, appId]);
}

/** Encrypted status per device as last read (P2); a device without an entry was never watched. */
export function useFleetDeviceStates(): Record<string, FleetDeviceState> {
	return useAttentionState().input.fleet;
}

export type { Coverage };
