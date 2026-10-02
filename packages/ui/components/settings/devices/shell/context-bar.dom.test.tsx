import { afterAll, afterEach, expect, test } from "bun:test";
import type { MemoryNavigation } from "../routing/use-devices-route";
import { byRole, click, installDom, queryByRole } from "../testing/dom-harness";

const dom = installDom();
const { MemoryDevicesRoute } = await import("../routing/use-devices-route");
const { ContextBar } = await import("./context-bar");

afterEach(dom.cleanup);
afterAll(dom.restore);

const APP = "app_invoice_ai";
const DEVICE = "6f1c2a3b-0d4e-4f5a-8b6c-7d8e9f0a1b2c";

async function mount(search: string, canShowWholeDevice: boolean) {
	const navigations: MemoryNavigation[] = [];
	const view = await dom.render(
		<MemoryDevicesRoute
			host="app"
			initialSearch={`id=${APP}&${search}`}
			onNavigate={(entry) => navigations.push(entry)}
		>
			<ContextBar
				appName="Invoice AI"
				canShowWholeDevice={canShowWholeDevice}
			/>
		</MemoryDevicesRoute>,
	);
	return { navigations, view };
}

test("a device opened from an app names the app and links back to it", async () => {
	const { navigations, view } = await mount(
		`device=${DEVICE}&tab=services`,
		true,
	);
	expect(view.container.textContent).toContain("App: Invoice AI");
	expect(view.container.textContent).toContain(
		"Services, metrics and activity are filtered to this app.",
	);
	const app = byRole("link", "Invoice AI");
	expect(app.getAttribute("href")).toBe(`/library/config/devices?id=${APP}`);
	await click(app);
	expect(navigations).toEqual([
		{ mode: "push", href: `/library/config/devices?id=${APP}` },
	]);
});

test("Show whole device opens the same object in the account area", async () => {
	const { navigations } = await mount(
		`device=${DEVICE}&service=invoice-extractor&tab=metrics`,
		true,
	);
	await click(byRole("button", "Show whole device"));
	expect(navigations).toEqual([
		{
			mode: "push",
			href: `/settings/devices?device=${DEVICE}&service=invoice-extractor&tab=metrics`,
		},
	]);
});

test("without whole-device access the way out is not offered", async () => {
	await mount(`device=${DEVICE}`, false);
	expect(queryByRole("button", "Show whole device")).toBeNull();
	expect(byRole("link", "Invoice AI")).toBeTruthy();
});

test("the app's own page has no context bar", async () => {
	const { view } = await mount("by=event", true);
	expect(view.container.innerHTML).toBe("");
});
