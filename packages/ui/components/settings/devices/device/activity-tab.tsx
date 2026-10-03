"use client";

import { ActivityView } from "../observe/activity-view";
import type { DeviceTabProps } from "../screen-props";

/** N2 › Activity & logs: the whole device, also when the page is opened from an app. */
export function DeviceActivityTab({
	deviceId,
	scope,
}: Readonly<DeviceTabProps>) {
	return <ActivityView deviceId={deviceId} scope={scope} />;
}
