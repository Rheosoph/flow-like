import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type {
	DevicesRoute,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import type { DevicesT } from "../primitives/area-context";
import type { NavigateOptions } from "../routing/use-devices-route";
import { installDom } from "../testing/dom-harness";
import type { DevicesSpotlightInput } from "./use-devices-spotlight";

const dom = installDom();
const { getI18n } = await import("@flow-like/locales");
const { useSpotlightStore } = await import("../../../../state/spotlight-state");
const { DEVICES_SPOTLIGHT_GROUP, buildSpotlightItems, useDevicesSpotlight } =
	await import("./use-devices-spotlight");

afterEach(dom.cleanup);
afterAll(dom.restore);

const t = getI18n().getFixedT("en", "devices") as DevicesT;
const ACCOUNT: DevicesScope = { kind: "account" };

function input(patch: Partial<DevicesSpotlightInput> = {}) {
	const navigations: { route: DevicesRoute; options?: NavigateOptions }[] = [];
	const value: DevicesSpotlightInput = {
		scope: ACCOUNT,
		devices: [
			{ id: "dev-1", name: "edge-berlin-01", note: "Needs attention" },
			{ id: "dev-2", name: "warehouse-pi", note: "Critical" },
		],
		services: [
			{
				deviceId: "dev-1",
				serviceId: "invoice-extractor",
				deviceName: "edge-berlin-01",
				appName: "Invoice AI",
			},
		],
		apps: [{ id: "app_invoice_ai", name: "Invoice AI" }],
		navigate: (route, options) => navigations.push({ route, options }),
		...patch,
	};
	return { value, navigations };
}

function Registered({
	value,
}: Readonly<{ value: DevicesSpotlightInput | null }>) {
	useDevicesSpotlight(value);
	return null;
}

const registered = () =>
	useSpotlightStore.getState().dynamicItems.get(DEVICES_SPOTLIGHT_GROUP);
const group = () =>
	useSpotlightStore
		.getState()
		.groups.find((entry) => entry.id === DEVICES_SPOTLIGHT_GROUP);

describe("buildSpotlightItems", () => {
	test("pages and actions are listed; devices, services and apps are found by search only", () => {
		const { value } = input();
		const items = buildSpotlightItems(value, t);
		expect(items.map((item) => item.label)).toEqual([
			"Fleet overview",
			"Services on devices",
			"Device access",
			"Device certificates",
			"Device keys & recovery",
			"Hub status for devices",
			"Set up a device",
			"Deploy an app to devices",
			"Request shared access to a device",
		]);
		for (const item of items) expect(item.group).toBe(DEVICES_SPOTLIGHT_GROUP);
		const [fleet, services] = items;
		expect(fleet?.subItems?.map((item) => item.label)).toEqual([
			"edge-berlin-01",
			"warehouse-pi",
		]);
		expect(services?.subItems?.map((item) => item.label)).toEqual([
			"invoice-extractor",
			"Invoice AI",
		]);
		expect(services?.subItems?.[0]?.description).toBe(
			"Invoice AI on edge-berlin-01",
		);
	});

	test("the new keywords reach Access, Keys & recovery and Hub status (S01)", () => {
		const items = buildSpotlightItems(input().value, t);
		const keywords = (label: string) =>
			items.find((item) => item.label === label)?.keywords ?? [];
		expect(keywords("Device access")).toEqual(
			expect.arrayContaining(["access", "share"]),
		);
		expect(keywords("Device keys & recovery")).toEqual(
			expect.arrayContaining(["keys", "recovery"]),
		);
		expect(keywords("Hub status for devices")).toContain("hub status");
		expect(keywords("Services on devices")).toContain("services");
	});

	test("ids are unique and stable, so usage counts survive re-registration", () => {
		const first = buildSpotlightItems(input().value, t);
		const second = buildSpotlightItems(input().value, t);
		const ids = (items: typeof first) =>
			items.flatMap((item) => [
				item.id,
				...(item.subItems ?? []).map((sub) => sub.id),
			]);
		expect(ids(first)).toEqual(ids(second));
		expect(new Set(ids(first)).size).toBe(ids(first).length);
	});

	test("items open their object, in the right scope", () => {
		const { value, navigations } = input();
		const items = buildSpotlightItems(value, t);
		const all = items.flatMap((item) => [item, ...(item.subItems ?? [])]);
		const run = (label: string) =>
			all.find((item) => item.label === label)?.action();
		run("warehouse-pi");
		run("invoice-extractor");
		run("Invoice AI");
		run("Device keys & recovery");
		run("Request shared access to a device");
		expect(navigations).toEqual([
			{
				route: { screen: "device", deviceId: "dev-2", tab: "overview" },
				options: undefined,
			},
			{
				route: {
					screen: "service",
					deviceId: "dev-1",
					serviceId: "invoice-extractor",
				},
				options: undefined,
			},
			{
				route: { screen: "app-devices", by: "device" },
				options: { scope: { kind: "app", appId: "app_invoice_ai" } },
			},
			{ route: { screen: "keys" }, options: { scope: ACCOUNT } },
			{
				route: { screen: "access", tab: "shared", action: "request" },
				options: { scope: ACCOUNT },
			},
		]);
	});

	test("lock and unlock actions exist only when they can do something", () => {
		const labels = (value: DevicesSpotlightInput) =>
			buildSpotlightItems(value, t).map((item) => item.label);
		expect(labels(input().value)).not.toContain("Lock all devices");
		expect(labels(input().value)).not.toContain("Unlock several devices…");
		let locked = 0;
		let unlocked = 0;
		const items = buildSpotlightItems(
			input({ onLockAll: () => locked++, onUnlockSeveral: () => unlocked++ })
				.value,
			t,
		);
		items.find((item) => item.label === "Lock all devices")?.action();
		items.find((item) => item.label === "Unlock several devices…")?.action();
		expect([locked, unlocked]).toEqual([1, 1]);
	});

	test("in an app, Deploy starts app-first", () => {
		const { value, navigations } = input({
			scope: { kind: "app", appId: "app_invoice_ai" },
		});
		buildSpotlightItems(value, t)
			.find((item) => item.label === "Deploy an app to devices")
			?.action();
		expect(navigations[0]?.route).toEqual({
			screen: "deploy",
			deviceIds: [],
			mode: "new",
		});
	});
});

describe("useDevicesSpotlight", () => {
	test("the group exists only while the area is mounted (P9)", async () => {
		expect(registered()).toBeUndefined();
		const view = await dom.render(<Registered value={input().value} />);
		expect(group()).toEqual({
			id: DEVICES_SPOTLIGHT_GROUP,
			label: "Devices",
			priority: 80,
		});
		expect(registered()).toHaveLength(9);
		await view.unmount();
		expect(registered()).toBeUndefined();
		expect(group()).toBeUndefined();
	});

	test("an area gate registers nothing", async () => {
		await dom.render(<Registered value={null} />);
		expect(registered()).toBeUndefined();
		expect(group()).toBeUndefined();
	});

	test("a new list replaces the old one", async () => {
		const view = await dom.render(<Registered value={input().value} />);
		const before = registered();
		await view.rerender(
			<Registered value={input({ devices: [], services: [] }).value} />,
		);
		expect(registered()).not.toBe(before);
		expect(registered()?.[0]?.subItems).toEqual([]);
	});
});
