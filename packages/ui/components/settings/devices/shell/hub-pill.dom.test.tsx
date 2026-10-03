import { afterAll, afterEach, expect, test } from "bun:test";
import type { HubDeviceSupport } from "../../../../lib/device-management/model/types";
import type { MemoryNavigation } from "../routing/use-devices-route";
import { byRole, click, installDom, queryByRole } from "../testing/dom-harness";

const dom = installDom();
const { MemoryDevicesRoute } = await import("../routing/use-devices-route");
const { HubPill } = await import("./hub-pill");

afterEach(dom.cleanup);
afterAll(dom.restore);

async function mount(state: HubDeviceSupport["state"], host?: "app") {
	const navigations: MemoryNavigation[] = [];
	const view = await dom.render(
		<MemoryDevicesRoute
			host={host ?? "account"}
			initialSearch={host ? "id=app_1" : ""}
			onNavigate={(entry) => navigations.push(entry)}
		>
			<HubPill state={state} />
		</MemoryDevicesRoute>,
	);
	return { navigations, view };
}

test("nothing shows while the hub supports devices", async () => {
	const { view } = await mount("on");
	expect(view.container.innerHTML).toBe("");
	expect(queryByRole("link")).toBeNull();
});

test("checking, unreachable and off are three different pills", async () => {
	const expected = {
		checking: ["Checking this hub…", "warning"],
		unreachable: ["Hub unreachable", "critical"],
		off: ["Devices are off on this hub", "critical"],
	} as const;
	for (const state of ["checking", "unreachable", "off"] as const) {
		const { view } = await mount(state);
		const [label, tone] = expected[state];
		const pill = byRole("link", label);
		expect(pill.getAttribute("data-hub")).toBe(state);
		expect(pill.querySelector("[data-tone]")?.getAttribute("data-tone")).toBe(
			tone,
		);
		expect(view.container.textContent).toBe(label);
		await dom.cleanup();
	}
});

test("the pill opens Hub status, keeps its name when the text hides and leaves the bar below 480 px", async () => {
	const { navigations } = await mount("unreachable");
	const pill = byRole("link", "Hub unreachable");
	expect(pill.getAttribute("href")).toBe("/settings/devices?view=hub");
	expect(pill.querySelector("[data-tone] span span")?.className).toContain(
		"@max-[900px]/devices:hidden",
	);
	expect(pill.className).toContain("@max-[480px]/devices:hidden");
	await click(pill);
	expect(navigations).toEqual([
		{ mode: "push", href: "/settings/devices?view=hub" },
	]);
});

test("in an app the pill leaves to the account area", async () => {
	await mount("off", "app");
	expect(
		byRole("link", "Devices are off on this hub").getAttribute("href"),
	).toBe("/settings/devices?view=hub");
});
