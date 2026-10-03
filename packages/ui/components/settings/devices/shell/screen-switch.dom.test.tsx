import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type {
	DevicesRoute,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import { installDom } from "../testing/dom-harness";
import type { ScreenComponents } from "./screen-switch";

const dom = installDom();
const { SCREENS, ScreenSwitch, railMode } = await import("./screen-switch");

afterEach(dom.cleanup);
afterAll(dom.restore);

const ACCOUNT: DevicesScope = { kind: "account" };
const APP: DevicesScope = { kind: "app", appId: "app_invoice_ai" };

/** Records what each screen received, so the mapping is tested without the real screens. */
function stubs() {
	const seen: { screen: string; props: Record<string, unknown> }[] = [];
	const stub = (screen: string) => (props: Record<string, unknown>) => {
		seen.push({ screen, props });
		return <p data-stub={screen} />;
	};
	const screens = Object.fromEntries(
		Object.keys(SCREENS).map((screen) => [screen, stub(screen)]),
	) as unknown as ScreenComponents;
	return { seen, screens };
}

const ROUTES: { route: DevicesRoute; scope: DevicesScope; extra?: object }[] = [
	{ route: { screen: "fleet", view: "services" }, scope: ACCOUNT },
	{
		route: { screen: "device", deviceId: "dev-1", tab: "keys" },
		scope: ACCOUNT,
		extra: { deviceId: "dev-1" },
	},
	{
		route: { screen: "service", deviceId: "dev-1", serviceId: "svc-1" },
		scope: APP,
		extra: { deviceId: "dev-1", serviceId: "svc-1" },
	},
	{ route: { screen: "setup", step: 2 }, scope: ACCOUNT },
	{ route: { screen: "deploy", deviceIds: ["dev-1"] }, scope: APP },
	{ route: { screen: "access", tab: "cloud" }, scope: ACCOUNT },
	{ route: { screen: "certificates" }, scope: ACCOUNT },
	{ route: { screen: "keys", guide: "new-computer" }, scope: ACCOUNT },
	{ route: { screen: "hub" }, scope: ACCOUNT },
	{
		route: { screen: "app-devices", by: "event" },
		scope: APP,
		extra: { appId: "app_invoice_ai" },
	},
];

test("every route screen has a component", () => {
	const screens = ROUTES.map(({ route }) => route.screen).sort();
	expect(Object.keys(SCREENS).sort()).toEqual(screens);
	for (const component of Object.values(SCREENS))
		expect(typeof component).toBe("function");
});

test("each route renders its screen with the route, the scope and its ids", async () => {
	for (const { route, scope, extra } of ROUTES) {
		const { seen, screens } = stubs();
		const view = await dom.render(
			<ScreenSwitch route={route} scope={scope} screens={screens} />,
		);
		const wrapper = view.container.firstElementChild;
		expect(wrapper?.getAttribute("data-screen")).toBe(route.screen);
		expect(
			wrapper?.querySelector("[data-stub]")?.getAttribute("data-stub"),
		).toBe(route.screen);
		expect(seen.at(-1)).toEqual({
			screen: route.screen,
			props: { route, scope, ...extra },
		});
		await dom.cleanup();
	}
});

describe("rail per screen (SPEC §3.1)", () => {
	const device: DevicesRoute = { screen: "device", deviceId: "dev-1" };
	const service: DevicesRoute = {
		screen: "service",
		deviceId: "dev-1",
		serviceId: "svc-1",
	};

	test("the fleet opens the rail as an overlay at every width", () => {
		const fleet: DevicesRoute = { screen: "fleet", view: "devices" };
		expect(railMode(fleet, ACCOUNT, true)).toBe("overlay");
		expect(railMode(fleet, ACCOUNT, false)).toBe("overlay");
	});

	test("device and service dock it when the area is wide", () => {
		for (const route of [device, service]) {
			expect(railMode(route, ACCOUNT, true)).toBe("docked");
			expect(railMode(route, ACCOUNT, false)).toBe("overlay");
		}
	});

	test("in an app the rail is always an overlay", () => {
		for (const route of [device, service])
			expect(railMode(route, APP, true)).toBe("overlay");
		expect(railMode({ screen: "app-devices", by: "device" }, APP, true)).toBe(
			"overlay",
		);
	});

	test("wizards and the account sections have none", () => {
		const none: DevicesRoute[] = [
			{ screen: "setup" },
			{ screen: "deploy", deviceIds: [] },
			{ screen: "access" },
			{ screen: "certificates" },
			{ screen: "keys" },
			{ screen: "hub" },
		];
		for (const route of none) {
			expect(railMode(route, ACCOUNT, true)).toBe("none");
			expect(railMode(route, APP, true)).toBe("none");
		}
	});
});
