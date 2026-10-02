import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { APPS } from "../../../lib/device-management/model/__fixtures__/apps";
import { SAMPLE_IDS } from "../../../lib/device-management/model/__fixtures__/sample-fleet";
import {
	byRole,
	byText,
	click,
	fire,
	installDom,
	queryByRole,
} from "./testing/dom-harness";
import type { MountDevicesOptions } from "./testing/mount-devices";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"./testing/mount-devices"
);
await preloadDevices();
const { act } = await import("react");
const { useSpotlightStore } = await import("../../../state/spotlight-state");
const registry = await import(
	"../../../lib/device-management/workspace/registry"
);
const { fakeDeviceApi } = await import("./testing/fake-device-api");
const { createFakeWorkspace } = await import("./testing/fake-workspace");
const { DEVICES_SPOTLIGHT_GROUP } = await import(
	"./shell/use-devices-spotlight"
);
const { useActivityTray } = await import("./shell/activity-tray");
const { DevicesArea, DevicesAreaSkeleton } = await import("./devices-area");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
	registry.dismissWorkspaceSwitch();
	useActivityTray.getState().setOpen(false);
	useSpotlightStore.getState().close();
});
afterAll(dom.restore);

const EDGE = SAMPLE_IDS.edge;
const APP = "app_invoice_ai";
const GONE = "99999999-8888-4777-8666-555555555555";

/** The host backend's app list; `mountDevices` supplies the rest. */
const appState = {
	getApps: async () =>
		Object.values(APPS).map((app) => [
			{ id: app.id, visibility: app.visibility },
			{ name: app.name, description: "" },
		]),
	getApp: async (appId: string) => ({
		id: appId,
		visibility: Object.values(APPS).find((app) => app.id === appId)?.visibility,
	}),
	getAppMeta: async (appId: string) => ({
		name: Object.values(APPS).find((app) => app.id === appId)?.name ?? appId,
		description: "",
	}),
};

interface AreaOptions extends MountDevicesOptions {
	scope?: "account" | "app";
}

async function mountArea(options: AreaOptions = {}) {
	const scope = options.scope ?? "account";
	const mounted = await mountDevices(null, {
		backend: { appState: appState as never },
		...options,
		host: scope,
		providers: false,
	});
	await mounted.rerender(
		<DevicesArea
			scope={scope}
			harness={{
				overrides: mounted.overrides,
				widthBucket: options.widthBucket ?? "wide",
				tickMs: false,
			}}
		/>,
	);
	await mounted.settle();
	return mounted;
}

const one = (selector: string) => dom.document.querySelector(selector);
const gate = () => one("[data-area-gate]")?.getAttribute("data-area-gate");
const screen = () => one("[data-screen]")?.getAttribute("data-screen");
const topbar = () => one('[data-shell="topbar"]') as HTMLElement;
const chrome = () =>
	Array.from(dom.document.querySelectorAll("[data-chrome]")).map((element) =>
		element.getAttribute("data-chrome"),
	);

describe("the frame on the sample fleet", () => {
	test("top bar, chrome, screen and status bar render with no gate", async () => {
		await mountArea();
		expect(gate()).toBeUndefined();
		expect(screen()).toBe("fleet");
		expect(one('[data-shell="area"]')?.getAttribute("data-frame")).toBe("page");
		expect(byRole("navigation", "Devices area", topbar())).toBeTruthy();
		expect(chrome()).toEqual(
			expect.arrayContaining(["attention", "keys", "activity", "planes"]),
		);
		expect(one("[data-hub]")).toBeNull();
	});

	test("nothing in the frame is fixed and the top bar holds no primary action", async () => {
		await mountArea();
		const area = one('[data-shell="area"]') as HTMLElement;
		for (const element of [area, ...Array.from(area.querySelectorAll("*"))])
			expect(element.getAttribute("class") ?? "").not.toMatch(
				/(^|\s|:)fixed(\s|$)/,
			);
		expect(topbar().querySelectorAll("[data-dv-primary]")).toHaveLength(0);
	});

	test("the top bar shows no codes or keys (R3)", async () => {
		await mountArea();
		const text = topbar().textContent ?? "";
		expect(text).not.toMatch(/\b[GD]\d{1,2}\b/);
		expect(text).not.toMatch(/\b[a-z]+_[a-z_]+\b/);
	});

	test("the skeleton is the same frame with nothing loaded", async () => {
		const view = await dom.render(<DevicesAreaSkeleton />);
		expect(view.container.querySelector('[data-shell="area"]')).toBeTruthy();
		expect(byRole("status").getAttribute("aria-busy")).toBe("true");
	});
});

describe("area gates (SPEC §3.12)", () => {
	test("signed out: Sign in, the frame stays, nothing is read", async () => {
		const mounted = await mountArea({ signedIn: false });
		expect(gate()).toBe("signed_out");
		expect(screen()).toBeUndefined();
		expect(topbar()).toBeTruthy();
		expect(chrome()).toEqual([]);
		await click(byRole("button", "Sign in"));
		expect(mounted.signIns()).toBe(1);
		expect(dom.document.querySelectorAll("[data-dv-primary]")).toHaveLength(1);
	});

	test("a token that can't manage devices is named as the reason", async () => {
		const fake = await createFakeWorkspace();
		fake.api.mode.tokenRestricted = true;
		await mountArea({ fake });
		expect(gate()).toBe("token");
		expect(byText("Your access token can't manage devices.")).toBeTruthy();
		expect(screen()).toBeUndefined();
	});

	test("an expired sign-in asks to sign in again", async () => {
		const fake = await createFakeWorkspace();
		fake.api.mode.signedIn = false;
		const mounted = await mountArea({ fake });
		expect(gate()).toBe("session_expired");
		await click(byRole("button", "Sign in again"));
		expect(mounted.signIns()).toBe(1);
	});

	test("checking shows the hub's name over a skeleton, then the page", async () => {
		const api = fakeDeviceApi();
		let release = () => {};
		const held = new Promise<void>((resolve) => {
			release = resolve;
		});
		const pending = (async (input: RequestInfo | URL) => {
			await held;
			return api.hubFetch(input);
		}) as typeof fetch;
		const fake = await createFakeWorkspace(undefined, {
			api,
			workspace: { fetch: pending },
		});
		const mounted = await mountArea({ fake });
		expect(gate()).toBe("checking");
		expect(byText("Checking hub.test…")).toBeTruthy();
		expect(one('[data-hub="checking"]')).toBeTruthy();
		expect(screen()).toBeUndefined();
		await act(async () => release());
		await mounted.settle();
		expect(gate()).toBeUndefined();
		expect(one("[data-hub]")).toBeNull();
	});

	test("devices off links to Hub status, which is not gated", async () => {
		const fake = await createFakeWorkspace();
		fake.api.mode.devicesEnabled = false;
		const mounted = await mountArea({ fake });
		expect(gate()).toBe("off");
		expect(byText("Devices aren't enabled on hub.test.")).toBeTruthy();
		expect(one('[data-hub="off"]')).toBeTruthy();
		await click(byRole("link", "Open hub status"));
		await mounted.settle();
		expect(mounted.navigations).toEqual([
			{ mode: "push", href: "/settings/devices?view=hub" },
		]);
		expect(gate()).toBeUndefined();
		expect(screen()).toBe("hub");
	});

	test("unreachable: Retry reports the attempt, and the page opens once the hub answers", async () => {
		const api = fakeDeviceApi();
		let answers = false;
		const hub = (async (input: RequestInfo | URL) =>
			answers
				? api.hubFetch(input)
				: new Response("Not Found", { status: 404 })) as typeof fetch;
		const fake = await createFakeWorkspace(undefined, {
			api,
			workspace: { fetch: hub },
		});
		const mounted = await mountArea({ fake });
		expect(gate()).toBe("unreachable");
		expect(
			byText("hub.test doesn't say whether it supports devices."),
		).toBeTruthy();
		expect(one('[data-hub="unreachable"]')).toBeTruthy();
		expect(screen()).toBeUndefined();
		await click(byRole("button", "Retry"));
		await mounted.settle();
		expect(one("[data-area-gate]")?.textContent).toMatch(
			/Still unreachable at \d{1,2}:\d{2}:\d{2}/,
		);
		answers = true;
		await click(byRole("button", "Retry"));
		await mounted.settle();
		expect(gate()).toBeUndefined();
		expect(screen()).toBe("fleet");
	});

	test("a hub that can't be reached is retried before the gate says so", async () => {
		const fake = await createFakeWorkspace();
		fake.api.mode.reachable = false;
		const mounted = await mountArea({ fake });
		expect(gate()).toBe("checking");
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 3_300));
		});
		await mounted.settle();
		expect(gate()).toBe("unreachable");
		expect(byText("Can't reach hub.test.")).toBeTruthy();
	}, 10_000);
});

describe("the app page", () => {
	test("shows the path instead of the area nav, in the config card's frame", async () => {
		await mountArea({ scope: "app", search: `id=${APP}` });
		expect(one('[data-shell="area"]')?.getAttribute("data-frame")).toBe("card");
		expect(queryByRole("navigation", "Devices area")).toBeNull();
		expect(byRole("navigation", "Breadcrumb", topbar()).textContent).toBe(
			"Devices",
		);
		expect(screen()).toBe("app-devices");
		expect(
			byRole("button", "Search this app's devices and services"),
		).toBeTruthy();
	});

	test("a device opened from the app names the app and offers the whole device", async () => {
		const mounted = await mountArea({
			scope: "app",
			search: `id=${APP}&device=${EDGE}&tab=services`,
		});
		expect(screen()).toBe("device");
		expect(byRole("navigation", "Breadcrumb", topbar()).textContent).toBe(
			"Devicesedge-berlin-01",
		);
		const bar = one('[data-shell="context-bar"]') as HTMLElement;
		expect(bar.textContent).toContain("App: Invoice AI");
		await click(byRole("button", "Show whole device", bar));
		expect(mounted.navigations.at(-1)).toEqual({
			mode: "push",
			href: `/settings/devices?device=${EDGE}&tab=services`,
		});
	});

	test("without an app it hands over to the account area", async () => {
		const mounted = await mountArea({ scope: "app", search: "" });
		expect(mounted.navigations).toEqual([
			{ mode: "replace", href: "/settings/devices" },
		]);
		expect(screen()).toBeUndefined();
		expect(gate()).toBeUndefined();
	});
});

describe("deep links", () => {
	test("a legacy device link lands on a tab once the fleet is loaded", async () => {
		const mounted = await mountArea({ search: `device=${EDGE}` });
		expect(mounted.navigations).toHaveLength(1);
		expect(mounted.navigations[0]?.mode).toBe("replace");
		expect(mounted.navigations[0]?.href).toMatch(
			new RegExp(
				`^/settings/devices\\?device=${EDGE}&tab=(certificates|overview)$`,
			),
		);
		expect(screen()).toBe("device");
		expect(one("[data-resolve]")).toBeNull();
	});

	test("an unknown device says why and opens the fleet", async () => {
		const mounted = await mountArea({ search: `device=${GONE}&tab=metrics` });
		expect(mounted.navigations).toEqual([
			{ mode: "replace", href: "/settings/devices" },
		]);
		expect(screen()).toBe("fleet");
		expect(one("[data-resolve]")?.textContent).toBe(
			`No device with the ID ${GONE} on hub.test. It may have been set up again or removed.`,
		);
		await click(byRole("button", "Dismiss"));
		expect(one("[data-resolve]")).toBeNull();
	});

	test("a resolvable link is left alone", async () => {
		const mounted = await mountArea({ search: `device=${EDGE}&tab=keys` });
		expect(mounted.navigations).toEqual([]);
		expect(screen()).toBe("device");
	});
});

describe("Spotlight and hotkeys", () => {
	const items = () =>
		useSpotlightStore.getState().dynamicItems.get(DEVICES_SPOTLIGHT_GROUP);

	test("the Devices group is registered only while the area is mounted", async () => {
		const mounted = await mountArea();
		const fleet = items()?.find((item) => item.label === "Fleet overview");
		expect(fleet?.subItems?.map((item) => item.label)).toContain(
			"edge-berlin-01",
		);
		expect(
			useSpotlightStore
				.getState()
				.groups.some((group) => group.id === DEVICES_SPOTLIGHT_GROUP),
		).toBe(true);
		await mounted.unmount();
		expect(items()).toBeUndefined();
	});

	test("an area gate registers nothing", async () => {
		await mountArea({ signedIn: false });
		expect(items()).toBeUndefined();
	});

	test("the search button opens the app's Spotlight", async () => {
		await mountArea();
		expect(useSpotlightStore.getState().isOpen).toBe(false);
		await click(byRole("button", "Search devices, services, actions"));
		expect(useSpotlightStore.getState().isOpen).toBe(true);
		useSpotlightStore.getState().close();
	});

	const slash = () =>
		new dom.window.KeyboardEvent("keydown", {
			key: "/",
			bubbles: true,
			cancelable: true,
		}) as unknown as KeyboardEvent;

	test("`/` focuses the docked rail's filter", async () => {
		await mountArea({ search: `device=${EDGE}&tab=overview` });
		const event = slash();
		await fire(dom.document.body, event);
		expect(event.defaultPrevented).toBe(true);
		expect(dom.document.activeElement?.hasAttribute("data-rail-filter")).toBe(
			true,
		);
	});

	test("`/` opens the overlay rail on the fleet", async () => {
		const mounted = await mountArea();
		const toggle = () => one('[data-shell="rail-toggle"]') as HTMLElement;
		expect(toggle().getAttribute("aria-expanded")).toBe("false");
		const event = slash();
		await fire(dom.document.body, event);
		await mounted.settle();
		expect(event.defaultPrevented).toBe(true);
		expect(toggle().getAttribute("aria-expanded")).toBe("true");
	});

	test("screens without a rail leave the key alone", async () => {
		await mountArea({ search: "view=hub" });
		expect(one('[data-shell="rail-toggle"]')).toBeNull();
		const event = slash();
		await fire(dom.document.body, event);
		expect(event.defaultPrevented).toBe(false);
	});
});

describe("page-level notices", () => {
	test("a profile switch that locked keys is said explicitly (S02)", async () => {
		const before = await createFakeWorkspace();
		registry.registerDeviceWorkspace(before.workspace);
		const unlocked = before.workspace.keys
			.list()
			.filter((session) => session.state === "unlocked").length;
		expect(unlocked).toBeGreaterThan(0);
		const after = await createFakeWorkspace(undefined, {
			profileId: "another-profile",
		});
		await act(async () => registry.registerDeviceWorkspace(after.workspace));

		await mountArea({ fake: after });
		const notice = one('[data-shell="switch-notice"]') as HTMLElement;
		expect(byText("Keys locked because you switched profiles.")).toBeTruthy();
		expect(notice.textContent).toContain(
			`Devices unlocked before: ${unlocked}.`,
		);
		await click(
			byRole("button", "Dismiss", notice.closest("[data-tone]") as HTMLElement),
		);
		expect(one('[data-shell="switch-notice"]')).toBeNull();
		await cleanupDevices();
		await registry.disposeDeviceWorkspace(after.workspace.scopeKey);
		await before.dispose();
	});

	test("a failing device list keeps the page and names the cause (R5)", async () => {
		const mounted = await mountArea();
		expect(one('[data-shell="hub-failing"]')).toBeNull();
		const restore = mounted.fake.api.fail({ method: "GET", path: "devices" });
		await act(async () => {
			await mounted.fake.queryClient.refetchQueries();
		});
		await mounted.settle();
		expect(gate()).toBeUndefined();
		expect(screen()).toBe("fleet");
		expect(byText("Couldn't refresh from the hub.")).toBeTruthy();
		expect(one('[data-shell="hub-failing"]')?.textContent).toMatch(
			/Showing data from \d{1,2}:\d{2}:\d{2}\./,
		);
		restore();
		await click(byRole("button", "Retry now"));
		await mounted.settle();
		expect(one('[data-shell="hub-failing"]')).toBeNull();
	});
});
