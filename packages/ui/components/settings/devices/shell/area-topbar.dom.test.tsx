import { afterAll, afterEach, expect, test } from "bun:test";
import type { DevicesRoute } from "../../../../lib/device-management/model/types";
import type { MemoryNavigation } from "../routing/use-devices-route";
import {
	allByRole,
	byRole,
	click,
	installDom,
	queryByRole,
} from "../testing/dom-harness";
import type { AreaTopbarProps } from "./area-topbar";

const dom = installDom();
const { MemoryDevicesRoute } = await import("../routing/use-devices-route");
const { AreaTopbar, appCrumbs } = await import("./area-topbar");

afterEach(dom.cleanup);
afterAll(dom.restore);

const APP = "app_invoice_ai";
const DEVICE = "6f1c2a3b-0d4e-4f5a-8b6c-7d8e9f0a1b2c";

async function mount(
	props: Partial<AreaTopbarProps> = {},
	host: "account" | "app" = "account",
	search = "",
) {
	const navigations: MemoryNavigation[] = [];
	let searches = 0;
	const view = await dom.render(
		<MemoryDevicesRoute
			host={host}
			initialSearch={host === "app" ? `id=${APP}&${search}` : search}
			onNavigate={(entry) => navigations.push(entry)}
		>
			<AreaTopbar rail={null} onSearch={() => searches++} {...props} />
		</MemoryDevicesRoute>,
	);
	const bar = view.container.querySelector(
		'[data-shell="topbar"]',
	) as HTMLElement;
	return { navigations, bar, searches: () => searches };
}

test("the account bar: home mark, area nav, search, chrome and trailing chips in order", async () => {
	const { bar } = await mount({
		children: <button type="button">Attention</button>,
		trailing: <span data-part="pill">Hub unreachable</span>,
	});
	expect(byRole("link", "Devices home", bar).getAttribute("href")).toBe(
		"/settings/devices",
	);
	expect(byRole("navigation", "Devices area", bar)).toBeTruthy();
	const search = byRole("button", "Search devices, services, actions", bar);
	const attention = byRole("button", "Attention", bar);
	const pill = bar.querySelector('[data-part="pill"]') as HTMLElement;
	const position = (element: Element) =>
		Array.from(bar.querySelectorAll("*")).indexOf(element);
	expect(position(search)).toBeLessThan(position(attention));
	expect(position(attention)).toBeLessThan(position(pill));
});

test("the bar is one 48 px line that never wraps, and hides labels below 720 px", async () => {
	const { bar } = await mount();
	expect(bar.className).toContain("h-12");
	expect(bar.className).toContain("whitespace-nowrap");
	expect(bar.className).toContain("overflow-hidden");
	expect(bar.className).not.toContain("flex-wrap");
	const search = byRole("button", "Search devices, services, actions", bar);
	expect(search.querySelector("span")?.className).toContain(
		"@max-[720px]/devices:hidden",
	);
	expect(search.querySelector("kbd")?.className).toContain(
		"@max-[720px]/devices:hidden",
	);
});

test("search opens the app's Spotlight through the handler", async () => {
	const { bar, searches } = await mount();
	await click(byRole("button", "Search devices, services, actions", bar));
	expect(searches()).toBe(1);
});

test("the rail toggle shows only for an overlay rail and reports its state", async () => {
	let toggles = 0;
	const { bar } = await mount({
		rail: { open: false, onToggle: () => toggles++ },
	});
	const toggle = byRole("button", "Show devices", bar);
	expect(toggle.getAttribute("aria-expanded")).toBe("false");
	await click(toggle);
	expect(toggles).toBe(1);
	await dom.cleanup();

	const open = await mount({ rail: { open: true, onToggle: () => {} } });
	expect(
		byRole("button", "Hide devices", open.bar).getAttribute("aria-expanded"),
	).toBe("true");
	await dom.cleanup();

	const none = await mount({ rail: null });
	expect(none.bar.querySelector('[data-shell="rail-toggle"]')).toBeNull();
});

test("the app bar shows the path instead of the area nav, and only the parent and the current page below 720 px", async () => {
	const { navigations, bar } = await mount(
		{
			crumbs: [
				{ label: "Devices", route: { screen: "app-devices", by: "device" } },
				{
					label: "edge-berlin-01",
					route: { screen: "device", deviceId: DEVICE, tab: "services" },
				},
				{ label: "invoice-extractor" },
			],
		},
		"app",
		`device=${DEVICE}&service=invoice-extractor`,
	);
	expect(queryByRole("navigation", "Devices area", bar)).toBeNull();
	expect(queryByRole("link", "Devices home", bar)).toBeNull();
	const crumbs = byRole("navigation", "Breadcrumb", bar);
	expect(
		allByRole("link", undefined, crumbs).map((a) => a.textContent),
	).toEqual(["Devices", "edge-berlin-01", "invoice-extractor"]);
	const current = crumbs.querySelector('[aria-current="page"]');
	expect(current?.textContent).toBe("invoice-extractor");
	const narrowHidden = (slot: string) =>
		Array.from(crumbs.querySelectorAll(`[data-slot="breadcrumb-${slot}"]`)).map(
			(node) => node.className.includes("@max-[720px]/devices:hidden"),
		);
	expect(narrowHidden("item")).toEqual([true, false, false]);
	expect(narrowHidden("separator")).toEqual([true, false]);
	expect(
		Array.from(
			crumbs.querySelectorAll('[data-slot="breadcrumb-item"]'),
			(node) => node.className.includes("min-w-0"),
		),
	).toEqual([false, true, true]);
	expect(byRole("link", "Devices", crumbs).getAttribute("href")).toBe(
		`/library/config/devices?id=${APP}`,
	);
	expect(
		byRole("button", "Search this app's devices and services", bar),
	).toBeTruthy();
	await click(byRole("link", "edge-berlin-01", crumbs));
	expect(navigations).toEqual([
		{
			mode: "push",
			href: `/library/config/devices?id=${APP}&device=${DEVICE}&tab=services`,
		},
	]);
});

test("a deploy that started on the Events page reads Events › Deploy and goes back there without a page load", async () => {
	const labels = {
		devices: "Devices",
		device: "",
		deploy: "Deploy",
		events: "Events",
	};
	const scope = { kind: "app", appId: APP } as const;
	const fromEvents: DevicesRoute = {
		screen: "deploy",
		deviceIds: [],
		mode: "new",
		eventId: "evt_visitor_page",
		from: "events",
		step: "what",
	};
	const events = `/library/config/events?id=${APP}&event=evt_visitor_page`;
	expect(appCrumbs(fromEvents, labels, scope)).toEqual([
		{ label: "Events", href: events },
		{ label: "Deploy" },
	]);
	const fromDevices: DevicesRoute = { screen: "deploy", deviceIds: [DEVICE] };
	expect(appCrumbs(fromDevices, labels, scope)).toEqual([
		{ label: "Devices", route: { screen: "app-devices", by: "device" } },
		{ label: "Deploy" },
	]);
	expect(appCrumbs(fromEvents, labels)[0]?.label).toBe("Devices");

	const { navigations, bar } = await mount(
		{ crumbs: appCrumbs(fromEvents, labels, scope) },
		"app",
		"flow=deploy&mode=new&event=evt_visitor_page&from=events&step=what",
	);
	const crumbs = byRole("navigation", "Breadcrumb", bar);
	expect(
		allByRole("link", undefined, crumbs).map((a) => a.textContent),
	).toEqual(["Events", "Deploy"]);
	const back = byRole("link", "Events", crumbs);
	expect(back.getAttribute("href")).toBe(events);
	await click(back);
	expect(navigations).toEqual([{ mode: "push", href: events }]);
});

test("the top bar has no primary button (R2)", async () => {
	const { bar } = await mount({ rail: { open: false, onToggle: () => {} } });
	expect(bar.querySelectorAll("[data-dv-primary]")).toHaveLength(0);
	expect(bar.innerHTML).not.toContain("bg-primary");
});
