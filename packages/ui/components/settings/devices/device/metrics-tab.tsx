"use client";

import { MetricsView } from "../observe/metrics-view";
import type { DeviceTabProps } from "../screen-props";

/** N2 › Metrics: the whole device, also when the page is opened from an app. */
export function DeviceMetricsTab({
	deviceId,
	scope,
}: Readonly<DeviceTabProps>) {
	return <MetricsView deviceId={deviceId} scope={scope} />;
}
