import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	SAMPLE_IDS,
	sampleFleet,
} from "../../../../lib/device-management/model/__fixtures__/sample-fleet";
import {
	advance,
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
	typeInto,
} from "../testing/dom-harness";
import type { MountDevicesOptions } from "../testing/mount-devices";

const dom = installDom();
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../testing/mount-devices"
);
await preloadDevices();
const { FleetScreen } = await import("./fleet-screen");
const { useDevicesRoute } = await import("../routing/use-devices-route");
const { useOverlayStore } = await import("../workspace/overlay-store");

async function resetAfterTest() {
	await cleanupDevices();
	await dom.cleanup();
	useOverlayStore.getState().close();
}
afterEach(resetAfterTest);
afterAll(dom.restore);

function Routed() {
	const { route, scope } = useDevicesRoute();
	return <FleetScreen route={route} scope={scope} />;
}

async function mountServices(options: MountDevicesOptions = {}) {
	const mounted = await mountDevices(<Routed />, {
		search: "view=services",
		...options,
	});
	// Owner names are looked up in one batch 20 ms after the rows render.
	await advance(40);
	await mounted.settle();
	return mounted;
}

function block(container: HTMLElement) {
	return container.querySelector("#devices-fleet-list") as HTMLElement;
}

function serviceIds(container: HTMLElement) {
	const ids: (string | undefined)[] = [];
	for (const row of block(container).querySelectorAll<HTMLElement>(
		"tr[data-service]",
	))
		ids.push(row.dataset.service);
	return ids;
}

function rowOf(container: HTMLElement, serviceId: string) {
	return block(container).querySelector(
		`tr[data-service="${serviceId}"]`,
	) as HTMLElement;
}

function chip(name: string) {
	return byRole("button", name, byRole("group", "Filter services"));
}

/** Picks an entry of the App select (a Radix listbox in a portal). */
async function pickApp(option: string | RegExp) {
	await click(byRole("combobox", "App"));
	await click(byRole("option", option, inPortal("listbox")));
}

describe("FleetServicesTable rows", () => {
	test("one row per readable service across devices, most severe first", async () => {
		const { container } = await mountServices();
		expect(serviceIds(container)).toEqual([
			"scanner-ingest",
			"invoice-extractor",
			"support-bot",
			"field-notes",
			"nightly-sync",
		]);
		expect(
			block(container).querySelector("[data-fleet-count]")?.textContent,
		).toBe("Showing 5 of 5 · most severe first");
		expect(
			byRole("button", /^Services/, byRole("group", "View")).getAttribute(
				"aria-pressed",
			),
		).toBe("true");
		expect(container.querySelectorAll("[data-dv-primary]").length).toBe(1);
	});

	test("a row names its app with how it runs and links to where the app runs on that device", async () => {
		const { container } = await mountServices();
		const scanner = rowOf(container, "scanner-ingest");
		expect(scanner.textContent).toContain("Warehouse Scanner");
		expect(scanner.textContent).toContain("· Offline copy");
		expect(
			byRole("link", "Warehouse Scanner", scanner).getAttribute("href"),
		).toBe(
			`/library/config/devices?id=app_warehouse_scan&focus=${SAMPLE_IDS.warehouse}`,
		);
		expect(byRole("link", "scanner-ingest", scanner).getAttribute("href")).toBe(
			`/settings/devices?device=${SAMPLE_IDS.warehouse}&service=scanner-ingest&tab=status`,
		);
		expect(scanner.textContent).toContain("Last known");
		expect(scanner.textContent).toMatch(/Crashing · .+, last known/);
		expect(scanner.textContent).toContain("0 of 1 ready");

		const invoice = rowOf(container, "invoice-extractor");
		expect(invoice.textContent).toContain("· Runs online");
		expect(invoice.textContent).toContain("Switching over");
		expect(invoice.textContent).toContain("safe update · v11 → v12");
		expect(invoice.textContent).toContain(
			"Settings v12 · running v11, applying",
		);
		expect(invoice.textContent).toContain("Cloud access");

		const notes = rowOf(container, "field-notes");
		expect(notes.querySelector('[data-badge="writes"]')?.textContent).toMatch(
			/^Write buffering · \d+ waiting · needs you$/,
		);
		expect(rowOf(container, "support-bot").textContent).toContain(
			"No update running",
		);
		expect(
			rowOf(container, "support-bot").querySelector('[data-badge="writes"]'),
		).toBeNull();
	});

	test("devices that can't be read are named under the table, with Unlock… where that helps", async () => {
		const { container } = await mountServices();
		const foot = block(container).querySelector(
			"[data-services-hidden]",
		) as HTMLElement;
		expect(foot.textContent).toContain("Not shown:");
		expect(foot.textContent).toContain("cold-storage-nas(no status yet)");
		expect(foot.textContent).toContain("lab-gpu-02(locked)");
		await click(byRole("button", "Unlock…", foot));
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "unlock",
			deviceId: SAMPLE_IDS.lab,
		});
	});

	test("with every device locked the view says so instead of looking empty", async () => {
		const { container } = await mountServices({ unlock: "none" });
		expect(serviceIds(container)).toEqual([]);
		const state = block(container).querySelector(
			'[data-kind="locked"]',
		) as HTMLElement;
		expect(state.textContent).toContain(
			"No device's services can be read on this computer yet",
		);
		expect(block(container).querySelector('[data-kind="empty"]')).toBeNull();
		await click(byRole("button", "Unlock several…", state));
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "unlock_several",
		});
	});

	test("a reader without the service list is named as not loaded; an authorised empty list just has no rows", async () => {
		const metricsOnly = sampleFleet();
		const warehouse = metricsOnly.fleet[SAMPLE_IDS.warehouse];
		warehouse.status = undefined;
		warehouse.saved = undefined;
		const first = await mountServices({ seed: metricsOnly });
		expect(serviceIds(first.container)).not.toContain("scanner-ingest");
		expect(
			block(first.container).querySelector("[data-services-hidden]")
				?.textContent,
		).toContain("warehouse-pi(not loaded)");
		await cleanupDevices();

		const empty = sampleFleet();
		const state = empty.fleet[SAMPLE_IDS.warehouse];
		for (const observation of state.status?.observations ?? [])
			observation.placements = [];
		state.saved = undefined;
		const second = await mountServices({ seed: empty });
		expect(serviceIds(second.container)).not.toContain("scanner-ingest");
		expect(
			block(second.container).querySelector("[data-services-hidden]")
				?.textContent,
		).not.toContain("warehouse-pi");
	});
});

describe("FleetServicesTable filters", () => {
	test("the chips narrow by state, write buffering and cloud access", async () => {
		const { container } = await mountServices();
		await click(chip("Crashing"));
		expect(serviceIds(container)).toEqual(["scanner-ingest"]);
		await click(chip("Not as requested"));
		expect(serviceIds(container)).toEqual([
			"scanner-ingest",
			"invoice-extractor",
		]);
		await click(chip("Updating"));
		expect(serviceIds(container)).toEqual(["invoice-extractor"]);
		await click(chip("Stopped"));
		expect(serviceIds(container)).toEqual(["nightly-sync"]);
		await click(chip("Has write buffering"));
		expect(serviceIds(container)).toContain("field-notes");
		expect(serviceIds(container)).not.toContain("support-bot");
		expect(serviceIds(container)).not.toContain("nightly-sync");
		await click(chip("Uses cloud access"));
		expect(serviceIds(container)).toEqual(["invoice-extractor", "field-notes"]);
		await click(chip("All"));
		expect(serviceIds(container).length).toBe(5);
	});

	test("a filter that leaves nothing offers the ways back", async () => {
		const { container } = await mountServices();
		await click(chip("Crashing"));
		await typeInto(byRole("textbox", "Search"), "nightly");
		expect(serviceIds(container)).toEqual([]);
		const state = block(container).querySelector(
			'[data-kind="empty"]',
		) as HTMLElement;
		expect(state.textContent).toContain("No service matches this filter");
		expect(state.textContent).toContain("5 services are readable right now.");
		expect(byRole("button", "Clear the search", state)).toBeTruthy();
		await click(byRole("button", "Show all states", state));
		expect(serviceIds(container)).toEqual(["nightly-sync"]);
	});

	test("search matches service, device and app", async () => {
		const { container, navigations } = await mountServices();
		const box = byRole("textbox", "Search");
		await typeInto(box, "studio");
		expect(serviceIds(container)).toEqual(["field-notes"]);
		await typeInto(box, "crm sync");
		expect(serviceIds(container)).toEqual(["nightly-sync"]);
		await advance(350);
		expect(navigations.at(-1)).toEqual({
			mode: "replace",
			href: "/settings/devices?view=services&q=crm+sync",
		});
	});

	test("the App filter narrows to one app, says how it runs and links to its Devices page", async () => {
		const { container, navigations } = await mountServices();
		await pickApp("Invoice AI · 1 service");
		expect(serviceIds(container)).toEqual(["invoice-extractor"]);
		// The filter is in the URL: a reload or a shared link keeps it.
		expect(navigations.at(-1)).toEqual({
			mode: "replace",
			href: "/settings/devices?view=services&app=app_invoice_ai",
		});
		const bar = block(container).querySelector(
			'[data-app-context="app_invoice_ai"]',
		) as HTMLElement;
		expect(bar.textContent).toContain("App: Invoice AI");
		expect(bar.textContent).toContain(
			"Runs online · showing its 1 service only",
		);
		expect(byRole("link", "Where it runs", bar).getAttribute("href")).toBe(
			"/library/config/devices?id=app_invoice_ai",
		);
		expect(byRole("link", "Deploy Invoice AI…", bar).getAttribute("href")).toBe(
			"/library/config/devices?id=app_invoice_ai&flow=deploy&mode=new",
		);
		await click(byRole("button", "Show all apps", bar));
		expect(serviceIds(container).length).toBe(5);
		expect(block(container).querySelector("[data-app-context]")).toBeNull();
		expect(navigations.at(-1)?.href).toBe("/settings/devices?view=services");
	});

	test("app=<id> in the link opens the Services view on that app", async () => {
		const { container } = await mountServices({
			search: "view=services&app=app_invoice_ai",
		});
		expect(serviceIds(container)).toEqual(["invoice-extractor"]);
		expect(
			block(container).querySelector('[data-app-context="app_invoice_ai"]'),
		).not.toBeNull();
	});

	test("an app that runs nowhere readable explains why and how it would run", async () => {
		const { container, settle } = await mountServices();
		await pickApp("Visitor Check-in · none readable");
		await settle();
		expect(serviceIds(container)).toEqual([]);
		const state = block(container).querySelector(
			'[data-kind="empty"]',
		) as HTMLElement;
		expect(state.textContent).toContain(
			"Visitor Check-in isn't on any device you can see",
		);
		expect(state.textContent).toContain(
			"lab-gpu-02 is locked, so its services aren't read here.",
		);
		expect(state.textContent).toMatch(
			/\d+ of its \d+ events can run on a device\./,
		);
		expect(
			byRole("link", "Open in the app's settings", state).getAttribute("href"),
		).toBe("/library/config/devices?id=app_visitor_checkin");
		expect(
			byRole("link", "Deploy Visitor Check-in…", state).getAttribute("href"),
		).toBe(
			"/library/config/devices?id=app_visitor_checkin&flow=deploy&mode=new",
		);
		await click(byRole("button", "Show all apps", state));
		expect(serviceIds(container).length).toBe(5);
	});
});

describe("FleetServicesTable actions", () => {
	test("the action follows the requested state", async () => {
		const { container } = await mountServices();
		const actions = (serviceId: string) =>
			[
				...rowOf(container, serviceId).querySelectorAll<HTMLElement>(
					"[data-service-action]",
				),
			].map((button) => button.dataset.serviceAction);
		expect(actions("support-bot")).toEqual(["restart", "stop"]);
		expect(actions("nightly-sync")).toEqual(["start"]);
		expect(actions("scanner-ingest")).toEqual(["start"]);
		expect(
			byRole("link", "Open", rowOf(container, "support-bot")).getAttribute(
				"href",
			),
		).toBe(
			`/settings/devices?device=${SAMPLE_IDS.edge}&service=support-bot&tab=status`,
		);
	});

	test("Stop… shows what it does in the row first, then sends one command", async () => {
		const { fake, container, settle } = await mountServices();
		const row = rowOf(container, "support-bot");
		const sent = fake.api.commands.length;
		await click(byRole("button", "Stop…", row));
		const confirm = block(container).querySelector(
			"tr[data-confirm-row]",
		) as HTMLElement;
		expect(confirm.textContent).toContain("Stop support-bot?");
		expect(confirm.textContent).toContain(
			"The device keeps it stopped until someone starts it.",
		);
		expect(fake.api.commands.length).toBe(sent);

		await click(byRole("button", "Stop support-bot", confirm));
		await settle();
		// The command once, then the status is read again.
		const after = fake.api.commands
			.slice(sent)
			.filter(([, type]) => type !== "inspect_page");
		expect(after).toEqual([
			[
				SAMPLE_IDS.edge,
				"stop",
				{ type: "stop", placement_id: "support-bot", expected_revision: 7 },
			],
		]);
		expect(block(container).querySelector("tr[data-confirm-row]")).toBeNull();
		expect(
			rowOf(container, "support-bot").querySelector("[data-result]"),
		).not.toBeNull();
	});

	test("Cancel closes the confirmation and sends nothing", async () => {
		const { fake, container } = await mountServices();
		const sent = fake.api.commands.length;
		await click(byRole("button", "Restart…", rowOf(container, "field-notes")));
		const confirm = block(container).querySelector(
			"tr[data-confirm-row]",
		) as HTMLElement;
		await click(byRole("button", "Cancel", confirm));
		expect(block(container).querySelector("tr[data-confirm-row]")).toBeNull();
		expect(fake.api.commands.length).toBe(sent);
	});

	test("a last-known row keeps its action visible, disabled, with the reason; clicking sends nothing", async () => {
		const { fake, container } = await mountServices();
		const row = rowOf(container, "scanner-ingest");
		const start = byRole("button", "Start", row);
		expect(start.getAttribute("aria-disabled")).toBe("true");
		expect(row.querySelector("[data-gate-inline]")?.textContent).toMatch(
			/warehouse-pi has been offline since .+\. This needs a live connection\./,
		);
		const sent = fake.api.commands.length;
		const calls = fake.api.calls.length;
		await click(start);
		expect(block(container).querySelector("tr[data-confirm-row]")).toBeNull();
		expect(fake.api.commands.length).toBe(sent);
		expect(fake.api.calls.length).toBe(calls);
	});

	test("during an update only Stop is allowed and the row says why", async () => {
		const { container } = await mountServices();
		const row = rowOf(container, "invoice-extractor");
		expect(
			byRole("button", "Restart…", row).getAttribute("aria-disabled"),
		).toBe("true");
		expect(byRole("button", "Stop…", row).getAttribute("aria-disabled")).toBe(
			null,
		);
		expect(row.querySelector("[data-gate-inline]")?.textContent).toContain(
			"Only Stop is allowed.",
		);
		expect(queryByRole("alert", undefined, container)).toBeNull();
	});
});
