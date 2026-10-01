import {
	ACCESS_TABS,
	type AccessRoute,
	type AppDevicesRoute,
	type AppRoute,
	CERTIFICATES_TABS,
	type CertificatesRoute,
	DEPLOY_STEP_IDS,
	DEVICE_TABS,
	type DeployRoute,
	type DeviceRoute,
	type DevicesRoute,
	type DevicesScope,
	FLEET_FILTERS,
	type FleetRoute,
	type KeysRoute,
	SERVICE_TABS,
	SETUP_STEPS,
	type ServiceRoute,
	type SetupRoute,
} from "../../../../lib/device-management/model/types";

/** The page that hosts the area: `/settings/devices` or `/library/config/devices`. */
export type DevicesHost = DevicesScope["kind"];

/** `URLSearchParams` and Next's `ReadonlyURLSearchParams` both fit. */
export interface ParamReader {
	get(name: string): string | null;
	getAll(name: string): string[];
}

export interface ParsedDevicesRoute {
	scope: DevicesScope;
	route: DevicesRoute;
}

export type NavigationMode = "push" | "replace";

export const ACCOUNT_SCOPE: DevicesScope = { kind: "account" };

const ROUTE_ID = /^[A-Za-z0-9_:.-]{1,128}$/;
const DEVICE_ACTIONS = ["revoke", "rename"] as const;
const DEPLOY_MODES = ["new", "update"] as const;
const ACCESS_IMPORTS = ["request", "connection"] as const;
const ACCESS_ACTIONS = ["add-people", "request"] as const;
const KEYS_GUIDES = ["new-computer", "forgot-password"] as const;
const ACCOUNT_ONLY_SCREENS: ReadonlySet<DevicesRoute["screen"]> = new Set([
	"fleet",
	"setup",
	"access",
	"certificates",
	"keys",
	"hub",
]);

/** Device, service (placement), app, event, enrollment and certificate ids: `[A-Za-z0-9-_:.]{1,128}`. */
export function isRouteId(value: unknown): value is string {
	return typeof value === "string" && ROUTE_ID.test(value);
}

export function isAppRoute(route: DevicesRoute): route is AppRoute {
	return !ACCOUNT_ONLY_SCREENS.has(route.screen);
}

/** Account-only screens always open in the account area; an app-first deploy carries its own app. */
export function routeScope(
	route: DevicesRoute,
	scope: DevicesScope,
): DevicesScope {
	if (scope.kind === "account") return scope;
	if (!isAppRoute(route)) return ACCOUNT_SCOPE;
	if (route.screen === "deploy" && route.appId && route.appId !== scope.appId)
		return { kind: "app", appId: route.appId };
	return scope;
}

function oneOf<T extends string | number>(
	values: readonly T[],
	raw: string | null,
): T | undefined {
	if (raw === null) return undefined;
	return values.find((value) => String(value) === raw);
}

function idParam(params: ParamReader, name: string): string | undefined {
	const value = params.get(name);
	return isRouteId(value) ? value : undefined;
}

function textParam(params: ParamReader, name: string): string | undefined {
	const value = params.get(name)?.trim();
	return value ? value : undefined;
}

function compact<T extends object>(value: T): T {
	return Object.fromEntries(
		Object.entries(value).filter(([, entry]) => entry !== undefined),
	) as T;
}

function parseFleet(params: ParamReader): FleetRoute {
	return compact<FleetRoute>({
		screen: "fleet",
		view: params.get("view") === "services" ? "services" : "devices",
		filter: oneOf(FLEET_FILTERS, params.get("filter")),
		q: textParam(params, "q"),
		focus: params.get("focus") === "attention" ? "attention" : undefined,
	});
}

/** An absent device tab stays absent (legacy link, resolved later); an unknown one falls back to Overview. */
function parseObject(
	params: ParamReader,
): DeviceRoute | ServiceRoute | undefined {
	const deviceId = idParam(params, "device");
	if (!deviceId) return undefined;
	const service = params.get("service");
	if (service !== null) {
		if (!isRouteId(service))
			return { screen: "device", deviceId, tab: "services" };
		return compact<ServiceRoute>({
			screen: "service",
			deviceId,
			serviceId: service,
			tab: oneOf(SERVICE_TABS, params.get("tab")),
			stream: params.get("stream") === "errors" ? "errors" : undefined,
		});
	}
	const tab = params.get("tab");
	return compact<DeviceRoute>({
		screen: "device",
		deviceId,
		tab: tab === null ? undefined : (oneOf(DEVICE_TABS, tab) ?? "overview"),
		certificateId: idParam(params, "certificate"),
		action: oneOf(DEVICE_ACTIONS, params.get("action")),
	});
}

function parseSetup(params: ParamReader): SetupRoute {
	return compact<SetupRoute>({
		screen: "setup",
		enrollmentId: idParam(params, "enrollment"),
		step: oneOf(SETUP_STEPS, params.get("step")),
	});
}

function parseDeploy(params: ParamReader, appId?: string): DeployRoute {
	return compact<DeployRoute>({
		screen: "deploy",
		deviceIds: [...new Set(params.getAll("device").filter(isRouteId))],
		appId: appId ?? idParam(params, "app") ?? idParam(params, "project"),
		serviceId: idParam(params, "service"),
		mode: oneOf(DEPLOY_MODES, params.get("mode")),
		eventId: idParam(params, "event"),
		from: params.get("from") === "events" ? "events" : undefined,
		step: oneOf(DEPLOY_STEP_IDS, params.get("step")),
	});
}

function parseAccountRoute(params: ParamReader): DevicesRoute {
	const flow = params.get("flow");
	if (flow === "setup") return parseSetup(params);
	if (flow === "deploy") return parseDeploy(params);
	const object = parseObject(params);
	if (object) return object;
	switch (params.get("view")) {
		case "access":
			return compact<AccessRoute>({
				screen: "access",
				tab: oneOf(ACCESS_TABS, params.get("tab")),
				import: oneOf(ACCESS_IMPORTS, params.get("import")),
				action: oneOf(ACCESS_ACTIONS, params.get("action")),
			});
		case "certificates":
			return compact<CertificatesRoute>({
				screen: "certificates",
				tab: oneOf(CERTIFICATES_TABS, params.get("tab")),
				action:
					params.get("action") === "create-authority"
						? "create-authority"
						: undefined,
			});
		case "keys":
			return compact<KeysRoute>({
				screen: "keys",
				guide: oneOf(KEYS_GUIDES, params.get("guide")),
				focusDeviceId: idParam(params, "focus"),
			});
		case "hub":
			return { screen: "hub" };
		default:
			return parseFleet(params);
	}
}

function parseAppRoute(params: ParamReader, appId: string): AppRoute {
	if (params.get("flow") === "deploy") return parseDeploy(params, appId);
	const object = parseObject(params);
	if (object) return object;
	return compact<AppDevicesRoute>({
		screen: "app-devices",
		by: params.get("by") === "event" ? "event" : "device",
		focusDeviceId: idParam(params, "focus"),
		eventId: idParam(params, "event"),
		action: params.get("action") === "update-all" ? "update-all" : undefined,
	});
}

/** The only reader of Devices URLs (plan §2.6). The app host without a valid `id` reads as the account area. */
export function parseDevicesRoute(
	input: ParamReader | string,
	host: DevicesHost,
): ParsedDevicesRoute {
	const params = typeof input === "string" ? new URLSearchParams(input) : input;
	const appId = host === "app" ? idParam(params, "id") : undefined;
	if (appId) {
		return {
			scope: { kind: "app", appId },
			route: parseAppRoute(params, appId),
		};
	}
	return { scope: ACCOUNT_SCOPE, route: parseAccountRoute(params) };
}

type QueryEntry = readonly [name: string, value: string | number | undefined];
type RouteOf<S extends DevicesRoute["screen"]> = Extract<
	DevicesRoute,
	{ screen: S }
>;
type EntryWriter<R extends DevicesRoute> = (
	route: R,
	scope: DevicesScope,
) => readonly QueryEntry[];

/** Param order per screen; `undefined` and `""` values are skipped, so defaults never reach the URL. */
const QUERY_ENTRIES: {
	[S in DevicesRoute["screen"]]: EntryWriter<RouteOf<S>>;
} = {
	fleet: (route) => [
		["view", route.view === "services" ? "services" : undefined],
		["filter", route.filter],
		["q", route.q?.trim()],
		["focus", route.focus],
	],
	device: (route) => [
		["device", route.deviceId],
		["tab", route.tab],
		["certificate", route.certificateId],
		["action", route.action],
	],
	service: (route) => [
		["device", route.deviceId],
		["service", route.serviceId],
		["tab", route.tab],
		["stream", route.stream],
	],
	setup: (route) => [
		["flow", "setup"],
		["enrollment", route.enrollmentId],
		["step", route.step],
	],
	deploy: (route, scope) => [
		["flow", "deploy"],
		["mode", route.mode],
		...route.deviceIds.map((deviceId): QueryEntry => ["device", deviceId]),
		["app", scope.kind === "account" ? route.appId : undefined],
		["service", route.serviceId],
		["event", route.eventId],
		["from", route.from],
		["step", route.step],
	],
	access: (route) => [
		["view", "access"],
		["tab", route.tab],
		["import", route.import],
		["action", route.action],
	],
	certificates: (route) => [
		["view", "certificates"],
		["tab", route.tab],
		["action", route.action],
	],
	keys: (route) => [
		["view", "keys"],
		["guide", route.guide],
		["focus", route.focusDeviceId],
	],
	hub: () => [["view", "hub"]],
	"app-devices": (route, scope) =>
		scope.kind === "account"
			? []
			: [
					["by", route.by === "event" ? "event" : undefined],
					["focus", route.focusDeviceId],
					["event", route.eventId],
					["action", route.action],
				],
};

function queryEntries(
	route: DevicesRoute,
	scope: DevicesScope,
): readonly QueryEntry[] {
	const write = QUERY_ENTRIES[route.screen] as EntryWriter<DevicesRoute>;
	return write(route, scope);
}

/** Query for the scope the route opens in (`routeScope`); the app scope writes `id` first, defaults are omitted. */
export function serializeDevicesRoute(
	route: DevicesRoute,
	scope: DevicesScope,
): URLSearchParams {
	const target = routeScope(route, scope);
	const params = new URLSearchParams();
	if (target.kind === "app") params.set("id", target.appId);
	for (const [name, value] of queryEntries(route, target)) {
		if (value !== undefined && value !== "") params.append(name, String(value));
	}
	return params;
}

function scopeKey(scope: DevicesScope): string {
	return scope.kind === "app" ? `app:${scope.appId}` : "account";
}

function deployKey(route: DeployRoute): string {
	const targets = [route.appId, route.serviceId, route.mode, route.eventId];
	return [
		"deploy",
		[...route.deviceIds].sort().join(","),
		...targets.map((value) => value ?? ""),
	].join(":");
}

function objectKey(route: DevicesRoute): string {
	switch (route.screen) {
		case "device":
			return `device:${route.deviceId}`;
		case "service":
			return `service:${route.deviceId}:${route.serviceId}`;
		case "setup":
			return `setup:${route.enrollmentId ?? ""}`;
		case "deploy":
			return deployKey(route);
		default:
			return route.screen;
	}
}

/** Moving between objects, sections or flows pushes; tab, filter, view, `by`, `step` and focus changes replace. */
export function navigationMode(
	from: ParsedDevicesRoute,
	to: ParsedDevicesRoute,
): NavigationMode {
	const sameScope =
		scopeKey(routeScope(from.route, from.scope)) ===
		scopeKey(routeScope(to.route, to.scope));
	return sameScope && objectKey(from.route) === objectKey(to.route)
		? "replace"
		: "push";
}
