import { afterAll, afterEach, expect, test } from "bun:test";
import type { DevicesRoute } from "../../../../lib/device-management/model/types";
import type { MemoryNavigation } from "../routing/use-devices-route";
import {
	allByRole,
	byRole,
	click,
	inPortal,
	installDom,
} from "../testing/dom-harness";

const dom = installDom();
const { MemoryDevicesRoute } = await import("../routing/use-devices-route");
const { AREA_SECTIONS, AreaNav, SECTION_ROUTE, areaSection } = await import(
	"./area-nav"
);

afterEach(dom.cleanup);
afterAll(dom.restore);

async function mount(search = "") {
	const navigations: MemoryNavigation[] = [];
	const view = await dom.render(
		<MemoryDevicesRoute
			initialSearch={search}
			onNavigate={(entry) => navigations.push(entry)}
		>
			<AreaNav />
		</MemoryDevicesRoute>,
	);
	const links = view.container.querySelector<HTMLElement>(
		'[data-nav="links"]',
	) as HTMLElement;
	const menu = view.container.querySelector<HTMLElement>(
		'[data-nav="menu"]',
	) as HTMLElement;
	return { navigations, links, menu };
}

test("fleet, device, service and both wizards belong to Devices", () => {
	const devices: DevicesRoute[] = [
		{ screen: "fleet", view: "services" },
		{ screen: "device", deviceId: "d" },
		{ screen: "service", deviceId: "d", serviceId: "s" },
		{ screen: "setup" },
		{ screen: "deploy", deviceIds: [] },
		{ screen: "app-devices", by: "device" },
	];
	for (const route of devices) expect(areaSection(route)).toBe("devices");
	for (const section of AREA_SECTIONS)
		expect(areaSection(SECTION_ROUTE[section])).toBe(section);
});

test("the five sections are links and the current one is marked", async () => {
	const { links } = await mount("view=certificates&tab=expiry");
	const names = allByRole("link", undefined, links).map(
		(link) => link.textContent,
	);
	expect(names).toEqual([
		"Devices",
		"Access",
		"Certificates",
		"Keys & recovery",
		"Hub status",
	]);
	expect(
		byRole("link", "Certificates", links).getAttribute("aria-current"),
	).toBe("page");
	expect(
		byRole("link", "Devices", links).getAttribute("aria-current"),
	).toBeNull();
	expect(byRole("link", "Hub status", links).getAttribute("href")).toBe(
		"/settings/devices?view=hub",
	);
	expect(byRole("navigation", "Devices area")).toBeTruthy();
});

test("a section link navigates inside the area", async () => {
	const { navigations, links } = await mount();
	await click(byRole("link", "Access", links));
	expect(navigations).toEqual([
		{ mode: "push", href: "/settings/devices?view=access" },
	]);
	expect(byRole("link", "Access", links).getAttribute("aria-current")).toBe(
		"page",
	);
});

test("links and the menu swap at 1080 px of area width, never both", async () => {
	const { links, menu } = await mount();
	expect(links.className).toContain("@max-[1080px]/devices:hidden");
	expect(menu.className).toContain("hidden");
	expect(menu.className).toContain("@max-[1080px]/devices:flex");
});

test("the collapsed menu names the current section, shrinks instead of overlapping and switches area", async () => {
	const { navigations, menu } = await mount("view=keys");
	const trigger = byRole("button", "Keys & recovery: switch area", menu);
	const classes = trigger.className.split(" ");
	expect(classes).toContain("shrink");
	expect(classes).not.toContain("shrink-0");
	expect(trigger.querySelector("span")?.className).toContain("truncate");
	await click(trigger);
	const items = allByRole("menuitem", undefined, inPortal("menu"));
	expect(items.map((item) => item.textContent)).toEqual([
		"Devices",
		"Access",
		"Certificates",
		"Keys & recovery",
		"Hub status",
	]);
	expect(
		byRole("menuitem", "Keys & recovery").getAttribute("aria-current"),
	).toBe("page");
	await click(byRole("menuitem", "Hub status"));
	expect(navigations).toEqual([
		{ mode: "push", href: "/settings/devices?view=hub" },
	]);
});
