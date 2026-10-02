import { describe, expect, test } from "bun:test";
import type {
	DevicesRoute,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import { appEventsHref, deployExitHref, devicesHref } from "./devices-href";
import {
	type DevicesHost,
	isRouteId,
	navigationMode,
	parseDevicesRoute,
	routeScope,
	serializeDevicesRoute,
} from "./devices-route";

const DEVICE = "6f1c2a3b-0d4e-4f5a-8b6c-7d8e9f0a1b2c";
const OTHER = "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d";
const ACCOUNT: DevicesScope = { kind: "account" };
const APP: DevicesScope = { kind: "app", appId: "app_invoice" };

function hostOf(scope: DevicesScope): DevicesHost {
	return scope.kind;
}

function urlParams(href: string): string {
	return href.includes("?") ? href.slice(href.indexOf("?") + 1) : "";
}

interface Row {
	name: string;
	href: string;
	scope: DevicesScope;
	route: DevicesRoute;
}

const ROWS: Row[] = [
	{
		name: "N1 devices view",
		href: "/settings/devices",
		scope: ACCOUNT,
		route: { screen: "fleet", view: "devices" },
	},
	{
		name: "N1 filter, search, attention focus",
		href: "/settings/devices?filter=critical&q=edge+berlin&focus=attention",
		scope: ACCOUNT,
		route: {
			screen: "fleet",
			view: "devices",
			filter: "critical",
			q: "edge berlin",
			focus: "attention",
		},
	},
	{
		name: "N1 services view",
		href: "/settings/devices?view=services&filter=locked",
		scope: ACCOUNT,
		route: { screen: "fleet", view: "services", filter: "locked" },
	},
	{
		name: "N1 services view of one app",
		href: "/settings/devices?view=services&app=app_invoice_ai&q=gpu",
		scope: ACCOUNT,
		route: {
			screen: "fleet",
			view: "services",
			app: "app_invoice_ai",
			q: "gpu",
		},
	},
	{
		name: "N1 shared with me",
		href: "/settings/devices?filter=shared",
		scope: ACCOUNT,
		route: { screen: "fleet", view: "devices", filter: "shared" },
	},
	{
		name: "N2 device tab",
		href: `/settings/devices?device=${DEVICE}&tab=metrics`,
		scope: ACCOUNT,
		route: { screen: "device", deviceId: DEVICE, tab: "metrics" },
	},
	{
		name: "N2 certificate focus (E12 reminder link)",
		href: `/settings/devices?device=${DEVICE}&tab=certificates&certificate=cert_01`,
		scope: ACCOUNT,
		route: {
			screen: "device",
			deviceId: DEVICE,
			tab: "certificates",
			certificateId: "cert_01",
		},
	},
	{
		name: "N2 revoke sheet",
		href: `/settings/devices?device=${DEVICE}&tab=settings&action=revoke`,
		scope: ACCOUNT,
		route: {
			screen: "device",
			deviceId: DEVICE,
			tab: "settings",
			action: "revoke",
		},
	},
	{
		name: "N2 legacy link keeps the tab open",
		href: `/settings/devices?device=${DEVICE}`,
		scope: ACCOUNT,
		route: { screen: "device", deviceId: DEVICE },
	},
	{
		name: "N3 service with errors stream (encoded id)",
		href: `/settings/devices?device=${DEVICE}&service=support-bot%3Ainvoice.v2&tab=activity&stream=errors`,
		scope: ACCOUNT,
		route: {
			screen: "service",
			deviceId: DEVICE,
			serviceId: "support-bot:invoice.v2",
			tab: "activity",
			stream: "errors",
		},
	},
	{
		name: "N5 set up resume",
		href: "/settings/devices?flow=setup&enrollment=enr_42&step=3",
		scope: ACCOUNT,
		route: { screen: "setup", enrollmentId: "enr_42", step: 3 },
	},
	{
		name: "N5 set up start",
		href: "/settings/devices?flow=setup",
		scope: ACCOUNT,
		route: { screen: "setup" },
	},
	{
		name: "N6 device-first deploy",
		href: `/settings/devices?flow=deploy&device=${DEVICE}&app=app_crm&service=crm-sync&step=settings`,
		scope: ACCOUNT,
		route: {
			screen: "deploy",
			deviceIds: [DEVICE],
			appId: "app_crm",
			serviceId: "crm-sync",
			step: "settings",
		},
	},
	{
		name: "N6 device-first deploy without an app",
		href: `/settings/devices?flow=deploy&device=${DEVICE}`,
		scope: ACCOUNT,
		route: { screen: "deploy", deviceIds: [DEVICE] },
	},
	{
		name: "N7 access",
		href: "/settings/devices?view=access&tab=shared&import=connection&action=request",
		scope: ACCOUNT,
		route: {
			screen: "access",
			tab: "shared",
			import: "connection",
			action: "request",
		},
	},
	{
		name: "N8 certificates",
		href: "/settings/devices?view=certificates&tab=authorities&action=create-authority",
		scope: ACCOUNT,
		route: {
			screen: "certificates",
			tab: "authorities",
			action: "create-authority",
		},
	},
	{
		name: "N9 keys",
		href: `/settings/devices?view=keys&guide=forgot-password&focus=${DEVICE}`,
		scope: ACCOUNT,
		route: {
			screen: "keys",
			guide: "forgot-password",
			focusDeviceId: DEVICE,
		},
	},
	{
		name: "N10 hub",
		href: "/settings/devices?view=hub",
		scope: ACCOUNT,
		route: { screen: "hub" },
	},
	{
		name: "App › Devices by device",
		href: "/library/config/devices?id=app_invoice",
		scope: APP,
		route: { screen: "app-devices", by: "device" },
	},
	{
		name: "App › Devices by event, focus, highlight, update all",
		href: `/library/config/devices?id=app_invoice&by=event&focus=${DEVICE}&event=evt_mail&action=update-all`,
		scope: APP,
		route: {
			screen: "app-devices",
			by: "event",
			focusDeviceId: DEVICE,
			eventId: "evt_mail",
			action: "update-all",
		},
	},
	{
		name: "N2 in app context",
		href: `/library/config/devices?id=app_invoice&device=${DEVICE}&tab=services`,
		scope: APP,
		route: { screen: "device", deviceId: DEVICE, tab: "services" },
	},
	{
		name: "N3 in app context",
		href: `/library/config/devices?id=app_invoice&device=${DEVICE}&service=invoice-extractor&tab=configuration`,
		scope: APP,
		route: {
			screen: "service",
			deviceId: DEVICE,
			serviceId: "invoice-extractor",
			tab: "configuration",
		},
	},
	{
		name: "N6 app-first deploy to several devices",
		href: `/library/config/devices?id=app_invoice&flow=deploy&mode=new&device=${DEVICE}&device=${OTHER}&step=where`,
		scope: APP,
		route: {
			screen: "deploy",
			deviceIds: [DEVICE, OTHER],
			appId: "app_invoice",
			mode: "new",
			step: "where",
		},
	},
	{
		name: "N6 single event from Events",
		href: "/library/config/devices?id=app_invoice&flow=deploy&mode=new&event=evt_mail&from=events",
		scope: APP,
		route: {
			screen: "deploy",
			deviceIds: [],
			appId: "app_invoice",
			mode: "new",
			eventId: "evt_mail",
			from: "events",
		},
	},
	{
		name: "N6 one-service update",
		href: `/library/config/devices?id=app_invoice&flow=deploy&mode=update&device=${DEVICE}&service=invoice-extractor&step=review`,
		scope: APP,
		route: {
			screen: "deploy",
			deviceIds: [DEVICE],
			appId: "app_invoice",
			serviceId: "invoice-extractor",
			mode: "update",
			step: "review",
		},
	},
];

describe("route round-trip (plan §2.6)", () => {
	test.each(ROWS.map((row) => [row.name, row] as const))("%s", (_name, row) => {
		const parsed = parseDevicesRoute(urlParams(row.href), hostOf(row.scope));
		expect(parsed.scope).toEqual(row.scope);
		expect(parsed.route).toStrictEqual(row.route);
		expect(devicesHref(row.route, row.scope)).toBe(row.href);
	});

	test("object routes round-trip in both scopes", () => {
		const routes: DevicesRoute[] = [
			{ screen: "device", deviceId: DEVICE, tab: "keys" },
			{
				screen: "service",
				deviceId: DEVICE,
				serviceId: "a_b-c:d.e",
				tab: "offline",
			},
		];
		for (const scope of [ACCOUNT, APP]) {
			for (const route of routes) {
				const params = serializeDevicesRoute(route, scope);
				const parsed = parseDevicesRoute(params, scope.kind);
				expect(parsed).toStrictEqual({ scope, route });
			}
		}
	});

	test("accepts a URLSearchParams-like reader", () => {
		const parsed = parseDevicesRoute(
			new URLSearchParams(`device=${DEVICE}&tab=access`),
			"account",
		);
		expect(parsed.route).toStrictEqual({
			screen: "device",
			deviceId: DEVICE,
			tab: "access",
		});
	});
});

describe("parsing rules", () => {
	test("project= is an alias of app=, the serializer writes app=", () => {
		const { route } = parseDevicesRoute(
			`flow=deploy&device=${DEVICE}&project=app_crm`,
			"account",
		);
		expect(route).toStrictEqual({
			screen: "deploy",
			deviceIds: [DEVICE],
			appId: "app_crm",
		});
		expect(devicesHref(route, ACCOUNT)).toBe(
			`/settings/devices?flow=deploy&device=${DEVICE}&app=app_crm`,
		);
	});

	test("the Services view's app filter: project= is an alias, and the Devices view has none", () => {
		const { route } = parseDevicesRoute(
			"view=services&project=app_crm",
			"account",
		);
		expect(route).toStrictEqual({
			screen: "fleet",
			view: "services",
			app: "app_crm",
		});
		expect(devicesHref(route, ACCOUNT)).toBe(
			"/settings/devices?view=services&app=app_crm",
		);
		expect(parseDevicesRoute("app=app_crm", "account").route).toStrictEqual({
			screen: "fleet",
			view: "devices",
		});
		expect(
			devicesHref(
				{ screen: "fleet", view: "devices", app: "app_crm" },
				ACCOUNT,
			),
		).toBe("/settings/devices");
		expect(
			parseDevicesRoute("view=services&app=not%20an%20id", "account").route,
		).toStrictEqual({ screen: "fleet", view: "services" });
	});

	test("app= wins over project=", () => {
		const { route } = parseDevicesRoute(
			"flow=deploy&app=app_a&project=app_b",
			"account",
		);
		expect(route).toMatchObject({ appId: "app_a" });
	});

	test.each<[string, string, DevicesRoute]>([
		[
			"unknown device tab opens Overview",
			`device=${DEVICE}&tab=bogus`,
			{ screen: "device", deviceId: DEVICE, tab: "overview" },
		],
		[
			"unknown service tab and stream are dropped",
			`device=${DEVICE}&service=svc&tab=bogus&stream=all`,
			{ screen: "service", deviceId: DEVICE, serviceId: "svc" },
		],
		[
			"unknown filter and focus are dropped",
			"filter=broken&focus=elsewhere&q=%20%20",
			{ screen: "fleet", view: "devices" },
		],
		[
			"unknown view opens the fleet",
			"view=nowhere",
			{ screen: "fleet", view: "devices" },
		],
		[
			"unknown device action is dropped",
			`device=${DEVICE}&tab=settings&action=explode`,
			{ screen: "device", deviceId: DEVICE, tab: "settings" },
		],
		["unknown setup step is dropped", "flow=setup&step=9", { screen: "setup" }],
		[
			"unknown deploy mode and step are dropped",
			`flow=deploy&device=${DEVICE}&mode=sideways&step=step-4&from=elsewhere`,
			{ screen: "deploy", deviceIds: [DEVICE] },
		],
		[
			"unknown section tabs are dropped",
			"view=access&tab=nobody&import=zip&action=delete",
			{ screen: "access" },
		],
		[
			"unknown keys guide is dropped",
			"view=keys&guide=lost-everything",
			{ screen: "keys" },
		],
		[
			"unknown flow reads the rest of the URL",
			"flow=teleport&view=hub",
			{ screen: "hub" },
		],
	])("%s", (_name, query, expected) => {
		expect(parseDevicesRoute(query, "account").route).toStrictEqual(expected);
	});

	test.each([
		["129 characters", "a".repeat(129)],
		["a slash", "svc/one"],
		["a space", "svc one"],
		["an empty id", ""],
	])(
		"a service id with %s is refused and opens the device's Services",
		(_name, id) => {
			const query = `device=${DEVICE}&service=${encodeURIComponent(id)}&tab=status`;
			expect(parseDevicesRoute(query, "account").route).toStrictEqual({
				screen: "device",
				deviceId: DEVICE,
				tab: "services",
			});
		},
	);

	test("a 128-character service id is accepted", () => {
		const id = "s".repeat(128);
		expect(
			parseDevicesRoute(`device=${DEVICE}&service=${id}`, "account").route,
		).toMatchObject({ screen: "service", serviceId: id });
	});

	test("isRouteId allows exactly [A-Za-z0-9-_:.]{1,128}", () => {
		expect(isRouteId("Az09-_:.")).toBe(true);
		expect(isRouteId("x".repeat(128))).toBe(true);
		expect(isRouteId("x".repeat(129))).toBe(false);
		expect(isRouteId("a%2Fb")).toBe(false);
		expect(isRouteId("ü")).toBe(false);
		expect(isRouteId(undefined)).toBe(false);
	});

	test("a malformed device id opens the fleet", () => {
		expect(
			parseDevicesRoute("device=..%2F..%2Fetc&tab=keys", "account").route,
		).toStrictEqual({ screen: "fleet", view: "devices" });
	});

	test("a malformed or repeated deploy target is dropped", () => {
		const { route } = parseDevicesRoute(
			`flow=deploy&device=${DEVICE}&device=bad%20id&device=${DEVICE}`,
			"account",
		);
		expect(route).toStrictEqual({ screen: "deploy", deviceIds: [DEVICE] });
	});

	test("flow wins over device, device wins over view", () => {
		expect(
			parseDevicesRoute(`flow=setup&device=${DEVICE}`, "account").route.screen,
		).toBe("setup");
		expect(
			parseDevicesRoute(`view=keys&device=${DEVICE}`, "account").route.screen,
		).toBe("device");
	});
});

describe("app scope", () => {
	test("without id the app host reads as the account area", () => {
		expect(parseDevicesRoute("", "app")).toStrictEqual({
			scope: ACCOUNT,
			route: { screen: "fleet", view: "devices" },
		});
		expect(parseDevicesRoute(`device=${DEVICE}&tab=keys`, "app")).toStrictEqual(
			{
				scope: ACCOUNT,
				route: { screen: "device", deviceId: DEVICE, tab: "keys" },
			},
		);
	});

	test("an invalid id reads as the account area", () => {
		expect(parseDevicesRoute("id=not%20an%20id", "app").scope).toEqual(ACCOUNT);
	});

	test("the account host ignores id", () => {
		expect(parseDevicesRoute("id=app_invoice", "account").scope).toEqual(
			ACCOUNT,
		);
	});

	test("account-only params in the app host fall back to App › Devices", () => {
		expect(
			parseDevicesRoute("id=app_invoice&view=keys&flow=setup", "app").route,
		).toStrictEqual({ screen: "app-devices", by: "device" });
	});

	test("app= in the app host is ignored: the app is the page's id", () => {
		expect(
			parseDevicesRoute("id=app_invoice&flow=deploy&app=app_other", "app")
				.route,
		).toMatchObject({ appId: "app_invoice" });
	});

	test("account-only screens link to the account area", () => {
		expect(devicesHref({ screen: "keys", guide: "new-computer" }, APP)).toBe(
			"/settings/devices?view=keys&guide=new-computer",
		);
		expect(routeScope({ screen: "hub" }, APP)).toEqual(ACCOUNT);
	});

	test("Show whole device: the same device route in the account scope", () => {
		const route: DevicesRoute = {
			screen: "device",
			deviceId: DEVICE,
			tab: "services",
		};
		expect(devicesHref(route, ACCOUNT)).toBe(
			`/settings/devices?device=${DEVICE}&tab=services`,
		);
	});

	test("an app-first deploy of another app opens in that app", () => {
		expect(
			devicesHref(
				{ screen: "deploy", deviceIds: [], appId: "app_crm", mode: "new" },
				APP,
			),
		).toBe("/library/config/devices?id=app_crm&flow=deploy&mode=new");
	});

	test("App › Devices without an app links to the fleet", () => {
		expect(devicesHref({ screen: "app-devices", by: "event" }, ACCOUNT)).toBe(
			"/settings/devices",
		);
	});
});

describe("deploy exit (from=events)", () => {
	test("from=events returns to the Events page with the event", () => {
		const { route, scope } = parseDevicesRoute(
			"id=app_invoice&flow=deploy&mode=new&event=evt_mail&from=events",
			"app",
		);
		if (route.screen !== "deploy") throw new Error("expected deploy");
		expect(deployExitHref(route, scope)).toBe(
			"/library/config/events?id=app_invoice&event=evt_mail",
		);
		expect(devicesHref(route, scope)).toContain("from=events");
	});

	test("from=events without an event returns to the Events list", () => {
		expect(
			deployExitHref(
				{
					screen: "deploy",
					deviceIds: [],
					appId: "app_invoice",
					from: "events",
				},
				APP,
			),
		).toBe("/library/config/events?id=app_invoice");
	});

	test("app-first exits to App › Devices", () => {
		expect(
			deployExitHref(
				{ screen: "deploy", deviceIds: [DEVICE], appId: "app_invoice" },
				APP,
			),
		).toBe("/library/config/devices?id=app_invoice");
	});

	test("device-first exits to the device's Services, or the service", () => {
		expect(
			deployExitHref({ screen: "deploy", deviceIds: [DEVICE] }, ACCOUNT),
		).toBe(`/settings/devices?device=${DEVICE}&tab=services`);
		expect(
			deployExitHref(
				{ screen: "deploy", deviceIds: [DEVICE], serviceId: "crm-sync" },
				ACCOUNT,
			),
		).toBe(`/settings/devices?device=${DEVICE}&service=crm-sync`);
		expect(
			deployExitHref({ screen: "deploy", deviceIds: [DEVICE, OTHER] }, ACCOUNT),
		).toBe("/settings/devices");
	});

	test("appEventsHref encodes ids", () => {
		expect(appEventsHref("app_invoice", "evt:1")).toBe(
			"/library/config/events?id=app_invoice&event=evt%3A1",
		);
	});
});

function at(route: DevicesRoute, scope: DevicesScope = ACCOUNT) {
	return { scope, route };
}

function device(tab: "overview" | "metrics") {
	return at({ screen: "device", deviceId: DEVICE, tab });
}

const NAVIGATION_CASES = [
	["tab change", device("overview"), device("metrics"), "replace"],
	[
		"filter change",
		at({ screen: "fleet", view: "devices" }),
		at({ screen: "fleet", view: "devices", filter: "offline" }),
		"replace",
	],
	[
		"view change",
		at({ screen: "fleet", view: "devices" }),
		at({ screen: "fleet", view: "services" }),
		"replace",
	],
	[
		"by change",
		at({ screen: "app-devices", by: "device" }, APP),
		at({ screen: "app-devices", by: "event" }, APP),
		"replace",
	],
	[
		"wizard step",
		at({ screen: "deploy", deviceIds: [DEVICE], step: "what" }),
		at({ screen: "deploy", deviceIds: [DEVICE], step: "where" }),
		"replace",
	],
	[
		"resolve cleanup of a certificate param",
		at({ screen: "device", deviceId: DEVICE, certificateId: "x" }),
		at({ screen: "device", deviceId: DEVICE, tab: "certificates" }),
		"replace",
	],
	[
		"fleet to device",
		at({ screen: "fleet", view: "devices" }),
		device("overview"),
		"push",
	],
	[
		"device to service",
		device("overview"),
		at({ screen: "service", deviceId: DEVICE, serviceId: "svc" }),
		"push",
	],
	[
		"device to another device",
		device("overview"),
		at({ screen: "device", deviceId: OTHER, tab: "overview" }),
		"push",
	],
	[
		"area section",
		at({ screen: "fleet", view: "devices" }),
		at({ screen: "keys" }),
		"push",
	],
	[
		"opening a wizard",
		device("overview"),
		at({ screen: "deploy", deviceIds: [DEVICE] }),
		"push",
	],
	[
		"app context to whole device",
		at({ screen: "device", deviceId: DEVICE, tab: "services" }, APP),
		at({ screen: "device", deviceId: DEVICE, tab: "services" }),
		"push",
	],
] as const;

describe("navigationMode", () => {
	test.each(NAVIGATION_CASES)("%s", (_name, from, to, mode) => {
		expect(navigationMode(from, to)).toBe(mode);
	});
});
