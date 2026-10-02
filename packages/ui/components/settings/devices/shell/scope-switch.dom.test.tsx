import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { APPS } from "../../../../lib/device-management/model/__fixtures__/apps";
import type { MemoryNavigation } from "../routing/use-devices-route";
import {
	allByRole,
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { MountDevicesOptions } from "../testing/mount-devices";
import type { ScopeApp } from "./scope-switch";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { MemoryDevicesRoute } = await import("../routing/use-devices-route");
const { SCOPE_MENU_CAP, ScopeSwitch, ScopeSwitchView } = await import(
	"./scope-switch"
);

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const APP_LIST: ScopeApp[] = [
	{ id: "app_invoice_ai", name: "Invoice AI" },
	{ id: "app_crm_sync", name: "CRM Sync" },
];

function Note({ appId }: Readonly<{ appId: string }>) {
	return <>note for {appId}</>;
}

async function mountView(
	apps: readonly ScopeApp[] | undefined,
	options: { host?: "app"; failed?: boolean } = {},
) {
	const navigations: MemoryNavigation[] = [];
	await dom.render(
		<MemoryDevicesRoute
			host={options.host ?? "account"}
			initialSearch={options.host ? "id=app_invoice_ai" : ""}
			onNavigate={(entry) => navigations.push(entry)}
		>
			<ScopeSwitchView apps={apps} failed={options.failed} AppNote={Note} />
		</MemoryDevicesRoute>,
	);
	return navigations;
}

const openMenu = async (name: string | RegExp = /^App/) => {
	await click(byRole("button", name, byRole("group", "Scope")));
	return inPortal("menu");
};

describe("ScopeSwitchView", () => {
	test("in the account area All devices is current and App opens a menu", async () => {
		await mountView(APP_LIST);
		const group = byRole("group", "Scope");
		const all = byRole("link", "All devices", group);
		expect(all.getAttribute("aria-current")).toBe("page");
		expect(all.getAttribute("href")).toBe("/settings/devices");
		const trigger = byRole("button", /^App/, group);
		expect(trigger.getAttribute("aria-current")).toBeNull();
		expect(trigger.getAttribute("aria-haspopup")).toBe("menu");
	});

	test("the menu lists each app with its note and opens the app's page", async () => {
		const navigations = await mountView(APP_LIST);
		const menu = await openMenu();
		const items = allByRole("menuitem", undefined, menu);
		expect(items.map((item) => item.textContent)).toEqual([
			"Invoice AInote for app_invoice_ai",
			"CRM Syncnote for app_crm_sync",
		]);
		expect(items[1]?.getAttribute("href")).toBe(
			"/library/config/devices?id=app_crm_sync",
		);
		await click(items[1] as HTMLElement);
		expect(navigations).toEqual([
			{ mode: "push", href: "/library/config/devices?id=app_crm_sync" },
		]);
	});

	test("in an app the second segment names the app and All devices leaves", async () => {
		await mountView(APP_LIST, { host: "app" });
		const group = byRole("group", "Scope");
		const all = byRole("link", "All devices", group);
		expect(all.getAttribute("aria-current")).toBeNull();
		expect(all.getAttribute("href")).toBe("/settings/devices");
		const trigger = byRole("button", /Invoice AI/, group);
		expect(trigger.getAttribute("aria-current")).toBe("page");
		const menu = await openMenu(/Invoice AI/);
		expect(
			byRole("menuitem", /^Invoice AI/, menu).getAttribute("aria-current"),
		).toBe("page");
	});

	test("many apps are capped and found with the filter (R11)", async () => {
		const many = Array.from({ length: 11 }, (_, index) => ({
			id: `app_${index}`,
			name: index === 10 ? "Zebra Scanner" : `Portal ${index}`,
		}));
		await mountView(many);
		const menu = await openMenu();
		expect(allByRole("menuitem", undefined, menu)).toHaveLength(SCOPE_MENU_CAP);
		expect(menu.textContent).toContain("3 more. Type to find an app.");
		await typeInto(byRole("textbox", "Filter apps", menu), "zebra");
		const found = allByRole("menuitem", undefined, inPortal("menu"));
		expect(found.map((item) => item.textContent)).toEqual([
			"Zebra Scannernote for app_10",
		]);
		await typeInto(byRole("textbox", "Filter apps"), "nothing like it");
		expect(inPortal("menu").textContent).toContain("No app matches that.");
	});

	test("few apps need no filter", async () => {
		await mountView(APP_LIST);
		const menu = await openMenu();
		expect(queryByRole("textbox", undefined, menu)).toBeNull();
	});

	test("loading, failed and empty lists say so instead of showing nothing", async () => {
		await mountView(undefined);
		expect((await openMenu()).textContent).toBe("Loading your apps…");
		await dom.cleanup();

		await mountView(undefined, { failed: true });
		expect((await openMenu()).textContent).toBe(
			"Your apps couldn't be loaded.",
		);
		await dom.cleanup();

		await mountView([]);
		expect((await openMenu()).textContent).toBe("You have no apps yet.");
	});
});

/** The app list of the host backend; `mountDevices` supplies everything else. */
function withApps(options: MountDevicesOptions = {}): MountDevicesOptions {
	const apps = Object.values(APPS);
	const find = (appId: string) => {
		const app = apps.find((entry) => entry.id === appId);
		if (!app) throw new Error(`No sample app ${appId}`);
		return app;
	};
	return {
		...options,
		backend: {
			appState: {
				getApps: async () =>
					apps.map((app) => [
						{ id: app.id, visibility: app.visibility },
						{ name: app.name, description: "" },
					]),
				getApp: async (appId: string) => ({
					id: appId,
					visibility: find(appId).visibility,
				}),
				getAppMeta: async (appId: string) => ({
					name: find(appId).name,
					description: "",
				}),
			} as never,
		},
	};
}

describe("ScopeSwitch on the sample fleet", () => {
	test("counts come from the hub's placement list where the hub has one", async () => {
		const { fake, settle } = await mountDevices(<ScopeSwitch />, withApps());
		await settle();
		const menu = await openMenu();
		await settle();
		const items = allByRole("menuitem", undefined, menu);
		expect(items).toHaveLength(
			Math.min(Object.keys(APPS).length, SCOPE_MENU_CAP),
		);
		for (const item of items)
			expect(item.textContent).toMatch(
				/(On \d+ devices you can see|Not on a device you can see)$/,
			);
		expect(menu.textContent).toMatch(/On [1-9]\d* devices you can see/);
		const placements = fake.api.sent("GET", /apps\/[^/]+\/device-placements/);
		expect(placements.length).toBe(items.length);
	});

	test("the placement lists are only read once the menu is open", async () => {
		const { fake, settle } = await mountDevices(<ScopeSwitch />, withApps());
		await settle();
		expect(fake.api.sent("GET", /device-placements/)).toEqual([]);
	});

	test("an older hub counts readable devices only and says so", async () => {
		const { fake, settle } = await mountDevices(
			<ScopeSwitch />,
			withApps({ hubVersion: "old" }),
		);
		await settle();
		const menu = await openMenu();
		await settle();
		const items = allByRole("menuitem", undefined, menu);
		for (const item of items)
			expect(item.textContent).toMatch(
				/(On \d+ of the \d+ devices you can read|Not on any of the \d+ devices you can read)$/,
			);
		expect(menu.textContent).not.toMatch(/devices you can see/);
		expect(menu.querySelector('[role="alert"]')).toBeNull();
		const perApp = new Map<string, number>();
		for (const [, path] of fake.api.sent("GET", /device-placements/))
			perApp.set(path, (perApp.get(path) ?? 0) + 1);
		expect(perApp.size).toBe(items.length);
		for (const count of perApp.values()) expect(count).toBe(1);
	});

	test("no machine vocabulary in the switch or its menu (R3)", async () => {
		const { settle } = await mountDevices(<ScopeSwitch />, withApps());
		await settle();
		const menu = await openMenu();
		await settle();
		const text = `${byRole("group", "Scope").textContent} ${menu.textContent}`;
		expect(text).not.toMatch(/\b[GD]\d{1,2}\b/);
		expect(text).not.toMatch(/\b[a-z]+_[a-z_]+\b/);
	});
});
