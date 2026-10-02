import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type {
	AttentionItem,
	DeviceRow,
	DevicesRoute,
} from "../../../../lib/device-management/model/types";
import { byRole, click, fire, installDom } from "../testing/dom-harness";
import type { DevicesHost } from "./devices-route";
import type { ResolveFleet } from "./resolve-target";
import type { DevicesRouteApi, MemoryNavigation } from "./use-devices-route";

const dom = installDom();
const { act } = await import("react");
const {
	DevicesRouteBoundary,
	MemoryDevicesRoute,
	useDevicesRoute,
	useRouteLink,
} = await import("./use-devices-route");

const NOW = 1_790_000_000;
const EDGE = "6f1c2a3b-0d4e-4f5a-8b6c-7d8e9f0a1b2c";
const GONE = "99999999-8888-4777-8666-555555555555";
const APP = "app_invoice_ai";

const row = (patch: Partial<DeviceRow> = {}): DeviceRow => ({
	device_id: EDGE,
	owner_id: "me",
	name: "edge-berlin-01",
	status: "active",
	registered_at: NOW - 86_400,
	last_seen_at: NOW,
	auth_epoch: 1,
	identity: {} as DeviceRow["identity"],
	...patch,
});

const fleet = (patch: Partial<ResolveFleet> = {}): ResolveFleet => ({
	devices: [row()],
	hub: "api.flow-like.com",
	...patch,
});

const certificateItem: Pick<AttentionItem, "key" | "subject"> = {
	key: "certificate_expiring",
	subject: { kind: "certificate", deviceId: EDGE, certificateId: "cert_01" },
};

let api: DevicesRouteApi;

function Probe() {
	api = useDevicesRoute();
	return <output data-screen={api.route.screen} />;
}

async function mount(search: string, host: DevicesHost = "account") {
	const navigations: MemoryNavigation[] = [];
	await dom.render(
		<MemoryDevicesRoute
			host={host}
			initialSearch={search}
			onNavigate={(navigation) => navigations.push(navigation)}
		>
			<Probe />
		</MemoryDevicesRoute>,
	);
	return navigations;
}

const go = (run: () => void) => act(async () => run());

afterEach(async () => {
	await dom.cleanup();
});
afterAll(dom.restore);

describe("route and scope from the URL", () => {
	test("the account page reads the object and its tab", async () => {
		await mount(`?device=${EDGE}&tab=metrics`);
		expect(api.host).toBe("account");
		expect(api.scope).toEqual({ kind: "account" });
		expect(api.route).toEqual({
			screen: "device",
			deviceId: EDGE,
			tab: "metrics",
		});
		expect(api.redirecting).toBe(false);
	});

	test("the app page reads the app scope", async () => {
		await mount(`id=${APP}&by=event`, "app");
		expect(api.scope).toEqual({ kind: "app", appId: APP });
		expect(api.route).toEqual({ screen: "app-devices", by: "event" });
	});

	test("the app page without an app hands over to the account area", async () => {
		const navigations = await mount(`device=${EDGE}&tab=keys`, "app");
		expect(api.redirecting).toBe(true);
		expect(navigations).toEqual([
			{ mode: "replace", href: `/settings/devices?device=${EDGE}&tab=keys` },
		]);
	});
});

describe("navigate", () => {
	test("pushes between objects and replaces within one", async () => {
		const navigations = await mount("");
		await go(() =>
			api.navigate({ screen: "device", deviceId: EDGE, tab: "overview" }),
		);
		expect(api.route.screen).toBe("device");
		await go(() =>
			api.navigate({ screen: "device", deviceId: EDGE, tab: "services" }),
		);
		await go(() => api.navigate({ screen: "keys" }));
		expect(navigations).toEqual([
			{ mode: "push", href: `/settings/devices?device=${EDGE}&tab=overview` },
			{
				mode: "replace",
				href: `/settings/devices?device=${EDGE}&tab=services`,
			},
			{ mode: "push", href: "/settings/devices?view=keys" },
		]);
	});

	test("the current target is not navigated to again", async () => {
		const navigations = await mount("view=keys");
		await go(() => api.navigate({ screen: "keys" }));
		expect(navigations).toEqual([]);
	});

	test("`replace` overrides the default in both directions", async () => {
		const navigations = await mount("");
		await go(() => api.navigate({ screen: "hub" }, { replace: true }));
		await go(() =>
			api.navigate({ screen: "fleet", view: "services" }, { replace: false }),
		);
		expect(navigations.map((entry) => entry.mode)).toEqual(["replace", "push"]);
	});

	test("another scope opens on its own page", async () => {
		const navigations = await mount("");
		const appHome: DevicesRoute = { screen: "app-devices", by: "device" };
		expect(api.href(appHome, { kind: "app", appId: APP })).toBe(
			`/library/config/devices?id=${APP}`,
		);
		await go(() =>
			api.navigate(appHome, { scope: { kind: "app", appId: APP } }),
		);
		expect(navigations).toEqual([
			{ mode: "push", href: `/library/config/devices?id=${APP}` },
		]);
	});

	test("an account section opened from an app leaves the app page", async () => {
		const navigations = await mount(`id=${APP}`, "app");
		await go(() => api.navigate({ screen: "hub" }));
		expect(navigations).toEqual([
			{ mode: "push", href: "/settings/devices?view=hub" },
		]);
	});
});

describe("clearParam", () => {
	test("drops one-shot params without a history entry", async () => {
		const navigations = await mount(
			`device=${EDGE}&tab=settings&action=revoke`,
		);
		await go(() => api.clearParam("action"));
		expect(navigations).toEqual([
			{
				mode: "replace",
				href: `/settings/devices?device=${EDGE}&tab=settings`,
			},
		]);
		expect(api.route).toEqual({
			screen: "device",
			deviceId: EDGE,
			tab: "settings",
		});
	});

	test("does nothing when the param is absent", async () => {
		const navigations = await mount("view=keys");
		await go(() => api.clearParam("guide", "focus"));
		expect(navigations).toEqual([]);
	});
});

describe("resolveWith", () => {
	test("a legacy device link opens Certificates when the device has a certificate item", async () => {
		const navigations = await mount(`device=${EDGE}`);
		await go(() => api.resolveWith(fleet({ attention: [certificateItem] })));
		expect(navigations).toEqual([
			{
				mode: "replace",
				href: `/settings/devices?device=${EDGE}&tab=certificates`,
			},
		]);
		expect(api.resolveBanner).toBeNull();
	});

	test("a legacy device link opens Overview otherwise", async () => {
		const navigations = await mount(`device=${EDGE}`);
		await go(() => api.resolveWith(fleet()));
		expect(navigations[0]?.href).toBe(
			`/settings/devices?device=${EDGE}&tab=overview`,
		);
	});

	test("an unknown device shows the reason and opens the fleet", async () => {
		const navigations = await mount(`device=${GONE}&tab=metrics`);
		await go(() => api.resolveWith(fleet()));
		expect(navigations).toEqual([
			{ mode: "replace", href: "/settings/devices" },
		]);
		expect(api.route.screen).toBe("fleet");
		expect(api.resolveBanner).toEqual({
			code: "device_not_found",
			params: { device: GONE, hub: "api.flow-like.com" },
		});
	});

	test("the banner stays through the fallback, and goes on dismiss or the next object", async () => {
		await mount(`device=${GONE}`);
		await go(() => api.resolveWith(fleet()));
		await go(() => api.resolveWith(fleet()));
		expect(api.resolveBanner?.code).toBe("device_not_found");
		await go(() => api.navigate({ screen: "fleet", view: "services" }));
		expect(api.resolveBanner?.code).toBe("device_not_found");
		await go(() => api.navigate({ screen: "hub" }));
		expect(api.resolveBanner).toBeNull();

		await go(() => api.navigate({ screen: "device", deviceId: GONE }));
		await go(() => api.resolveWith(fleet()));
		expect(api.resolveBanner).not.toBeNull();
		await go(() => api.dismissResolveBanner());
		expect(api.resolveBanner).toBeNull();
	});

	test("in an app, an unknown device falls back to the app's own page", async () => {
		const navigations = await mount(`id=${APP}&device=${GONE}`, "app");
		await go(() => api.resolveWith(fleet()));
		expect(navigations).toEqual([
			{ mode: "replace", href: `/library/config/devices?id=${APP}` },
		]);
		expect(api.route.screen).toBe("app-devices");
	});

	test("a resolvable target changes nothing", async () => {
		const navigations = await mount(`device=${EDGE}&tab=keys`);
		await go(() => api.resolveWith(fleet()));
		expect(navigations).toEqual([]);
		expect(api.resolveBanner).toBeNull();
	});
});

describe("useRouteLink", () => {
	function Link() {
		const link = useRouteLink();
		return (
			<a {...link({ screen: "certificates", tab: "expiry" })}>Certificates</a>
		);
	}

	async function mountLink() {
		const navigations: MemoryNavigation[] = [];
		await dom.render(
			<MemoryDevicesRoute onNavigate={(entry) => navigations.push(entry)}>
				<Link />
			</MemoryDevicesRoute>,
		);
		return navigations;
	}

	test("is a real link that navigates inside the area on a plain click", async () => {
		const navigations = await mountLink();
		const anchor = byRole("link", "Certificates");
		expect(anchor.getAttribute("href")).toBe(
			"/settings/devices?view=certificates&tab=expiry",
		);
		await click(anchor);
		expect(navigations).toEqual([
			{
				mode: "push",
				href: "/settings/devices?view=certificates&tab=expiry",
			},
		]);
	});

	test("leaves a modified click to the browser", async () => {
		const navigations = await mountLink();
		const anchor = byRole("link", "Certificates");
		const event = new dom.window.MouseEvent("click", {
			bubbles: true,
			cancelable: true,
			metaKey: true,
		}) as unknown as Event;
		await fire(anchor, event);
		expect(navigations).toEqual([]);
		expect(event.defaultPrevented).toBe(false);
	});
});

test("DevicesRouteBoundary uses the route above it instead of the Next router", async () => {
	await dom.render(
		<MemoryDevicesRoute initialSearch="view=hub">
			<DevicesRouteBoundary host="account">
				<Probe />
			</DevicesRouteBoundary>
		</MemoryDevicesRoute>,
	);
	expect(api.route).toEqual({ screen: "hub" });
});
