"use client";

import { MetricsView } from "../observe/metrics-view";
import type { ServiceTabProps } from "../screen-props";

/** N3 › Metrics: one service. */
export function ServiceMetricsTab({
	deviceId,
	serviceId,
	scope,
}: Readonly<ServiceTabProps>) {
	return (
		<MetricsView deviceId={deviceId} serviceId={serviceId} scope={scope} />
	);
}
