"use client";

import { ActivityView } from "../observe/activity-view";
import type { ServiceTabProps } from "../screen-props";

/** N3 › Activity & logs: one service; `stream=errors` opens its logs on Errors only. */
export function ServiceActivityTab({
	route,
	deviceId,
	serviceId,
	scope,
}: Readonly<ServiceTabProps>) {
	return (
		<ActivityView
			deviceId={deviceId}
			serviceId={serviceId}
			scope={scope}
			errorsFirst={route.screen === "service" && route.stream === "errors"}
		/>
	);
}
