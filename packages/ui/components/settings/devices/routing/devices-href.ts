import type {
	DeployRoute,
	DevicesRoute,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import { routeScope, serializeDevicesRoute } from "./devices-route";

export const ACCOUNT_DEVICES_PATH = "/settings/devices";
export const APP_DEVICES_PATH = "/library/config/devices";
export const APP_EVENTS_PATH = "/library/config/events";

function withQuery(path: string, params: URLSearchParams): string {
	const query = params.toString();
	return query ? `${path}?${query}` : path;
}

/** Every internal link, attention action and Spotlight item goes through here so the two bases never drift. */
export function devicesHref(route: DevicesRoute, scope: DevicesScope): string {
	const target = routeScope(route, scope);
	return withQuery(
		target.kind === "app" ? APP_DEVICES_PATH : ACCOUNT_DEVICES_PATH,
		serializeDevicesRoute(route, target),
	);
}

export function appEventsHref(appId: string, eventId?: string): string {
	const params = new URLSearchParams({ id: appId });
	if (eventId) params.set("event", eventId);
	return withQuery(APP_EVENTS_PATH, params);
}

/** Exit and Done of the deploy wizard: back to the Events page (`from=events`), App › Devices, or the device-first origin. */
export function deployExitHref(
	route: DeployRoute,
	scope: DevicesScope,
): string {
	const target = routeScope(route, scope);
	const appId = target.kind === "app" ? target.appId : route.appId;
	if (route.from === "events" && appId)
		return appEventsHref(appId, route.eventId);
	if (target.kind === "app")
		return devicesHref({ screen: "app-devices", by: "device" }, target);
	const [deviceId] = route.deviceIds;
	if (!deviceId || route.deviceIds.length > 1)
		return devicesHref({ screen: "fleet", view: "devices" }, target);
	return devicesHref(
		route.serviceId
			? { screen: "service", deviceId, serviceId: route.serviceId }
			: { screen: "device", deviceId, tab: "services" },
		target,
	);
}
