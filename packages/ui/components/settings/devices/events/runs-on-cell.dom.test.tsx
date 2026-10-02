import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	APPS,
	SERVICES,
	sampleDevices,
} from "../../../../lib/device-management/model/__fixtures__/apps";
import {
	allByRole,
	byRole,
	click,
	installDom,
	queryByRole,
} from "../testing/dom-harness";
import type { EventsDevicesValue } from "./events-devices";
import type { SampleApp, SampleValueOptions } from "./events-test-kit";

const dom = installDom();
const { RunsOnCell, RunsOnColumnHeader, runsOnReasonId } = await import(
	"./runs-on-cell"
);
const { SampleEventsDevices, sampleEvents, sampleValue } = await import(
	"./events-test-kit"
);
const { useOverlayStore } = await import("../workspace/overlay-store");

afterEach(async () => {
	useOverlayStore.getState().close();
	await dom.cleanup();
});
afterAll(dom.restore);

/** No snake_case wire value, gate code or event-rule code may reach the screen (R3). */
const MACHINE_WORDS =
	/\b[a-z]+_[a-z_]+\b|\bG\d{1,2}\b|latest_flow|api_type|not_served|cant_here/;

const nameOf = (appId: SampleApp, eventId: string) =>
	APPS[appId].events.find((event) => event.id === eventId)?.name ?? "";

async function renderCells(
	appId: SampleApp,
	options: SampleValueOptions & { compact?: boolean; both?: boolean } = {},
) {
	const value = sampleValue(appId, options);
	const view = await dom.render(
		<SampleEventsDevices value={value}>
			<ul>
				{sampleEvents(appId).map((event) => (
					<li key={event.id} data-event={event.id}>
						{options.compact ? null : (
							<RunsOnCell appId={appId} eventId={event.id} />
						)}
						{options.compact || options.both ? (
							<RunsOnCell appId={appId} eventId={event.id} compact />
						) : null}
					</li>
				))}
			</ul>
		</SampleEventsDevices>,
	);
	const cell = (eventId: string, compact = options.compact ?? false) => {
		const found = view.container.querySelector<HTMLElement>(
			`[id="${runsOnReasonId(eventId, compact)}"]`,
		);
		if (!found) throw new Error(`no cell for ${eventId}`);
		return found;
	};
	return { ...view, value, cell, text: (id: string) => cell(id).textContent };
}

const firstLine = (cell: HTMLElement) =>
	cell.firstElementChild?.firstElementChild?.textContent ??
	cell.firstElementChild?.textContent;

describe("Devices cell: where an event runs (APP §4.4)", () => {
	test("online: a device chip links to the service in app scope and says what it runs", async () => {
		const { cell } = await renderCells("app_invoice_ai");
		const served = cell("evt_extract_http");
		const chip = byRole("link", "edge-berlin-01", served);
		expect(chip.getAttribute("href")).toBe(
			"/library/config/devices?id=app_invoice_ai&device=edge-berlin-01&service=invoice-extractor",
		);
		expect(chip.getAttribute("title")).toBe(
			"invoice-extractor · Starting · 1.4.0 → 1.5.0 available",
		);
		expect(chip.querySelector("[data-tone]")?.getAttribute("data-tone")).toBe(
			"info",
		);
		expect(byRole("button", "1 locked", served)).toBeTruthy();
		expect(served.textContent).toContain("1 runs an older version");
	});

	test("online: an event on no readable device says so only for the devices it can see", async () => {
		const { cell } = await renderCells("app_invoice_ai");
		const none = cell("evt_gpu_extract");
		expect(none.textContent).toContain("Not on a device you can see");
		expect(byRole("button", "1 locked", none)).toBeTruthy();
		expect(byRole("link", "Run on a device…", none).getAttribute("href")).toBe(
			"/library/config/devices?id=app_invoice_ai&flow=deploy&mode=new&event=evt_gpu_extract&from=events",
		);
	});

	test("never reads “Not on a device” while any device is unknown", async () => {
		for (const appId of Object.keys(APPS) as SampleApp[]) {
			const { container, unmount } = await renderCells(appId);
			for (const cell of container.querySelectorAll<HTMLElement>(
				"[data-runs-on='row']",
			))
				expect(firstLine(cell)).not.toBe("Not on a device");
			await unmount();
		}
	});

	test("lab-unlocked: the shared device shows up as a chip with the older-version mark", async () => {
		const { cell } = await renderCells("app_invoice_ai", { labUnlocked: true });
		const gpu = cell("evt_gpu_extract");
		const chip = byRole("link", "lab-gpu-02", gpu);
		expect(chip.querySelector("svg")).toBeTruthy();
		expect(chip.querySelector("[data-tone]")?.getAttribute("data-tone")).toBe(
			"good",
		);
		expect(gpu.textContent).toContain("1 runs an older version");
		expect(queryByRole("button", /locked|unknown/, gpu)).toBeNull();
		const none = cell("evt_invoice_mcp");
		expect(firstLine(none)).toBe("Not on a device");
		expect(byRole("link", "Run on a device…", none)).toBeTruthy();
	});

	test("normal: a stopped service has the paused dot; events that can't run give the short reason", async () => {
		const { cell, text } = await renderCells("app_crm_sync");
		expect(
			cell("evt_crm_nightly")
				.querySelector("[data-tone]")
				?.getAttribute("data-tone"),
		).toBe("paused");
		expect(text("evt_crm_hourly")).toBe("Can't run on devicesSchedules can't");
		expect(text("evt_crm_rest")).toBe(
			"Can't run on devicesSplits traffic with a canary",
		);
		expect(
			queryByRole("link", "Run on a device…", cell("evt_crm_hourly")),
		).toBe(null);
	});

	test("local-only: both served events show the device; a paused one says to activate it", async () => {
		const { cell, text } = await renderCells("app_support_portal");
		expect(
			byRole("link", "edge-berlin-01", cell("evt_support_chat")),
		).toBeTruthy();
		expect(
			byRole("link", "edge-berlin-01", cell("evt_support_http")),
		).toBeTruthy();
		expect(text("evt_support_reply")).toBe(
			"Can't run on devicesOpens inside Flow-Like",
		);
		expect(text("evt_support_mailbox")).toBe("Paused: activate it first");
	});

	test("visitor: a never-deployed app offers Run on a device… for each event that can run", async () => {
		const { cell, text } = await renderCells("app_visitor_checkin", {
			labUnlocked: true,
		});
		for (const eventId of ["evt_visitor_page", "evt_badge_printer"]) {
			expect(firstLine(cell(eventId))).toBe("Not on a device");
			expect(byRole("link", "Run on a device…", cell(eventId))).toBeTruthy();
		}
		expect(text("evt_visitor_mail")).toBe(
			"Can't run on devicesMailboxes can't",
		);
	});

	test("three or more devices: two chips and a +N that opens the popover", async () => {
		const devices = sampleDevices().map((device) =>
			["studio-mac-mini", "warehouse-pi"].includes(device.id)
				? {
						...device,
						services: [{ ...SERVICES.invoiceExtractor, deviceId: device.id }],
					}
				: device,
		);
		const { cell } = await renderCells("app_invoice_ai", { devices });
		const served = cell("evt_extract_http");
		expect(served.querySelectorAll("[data-runs-on-chip]")).toHaveLength(2);
		await click(byRole("button", "+1", served));
		const dialog = byRole("dialog", "Where Extract invoice runs");
		expect(dialog.textContent).toContain("Serves it · 3");
	});

	test("a status without an event list counts as unknown, never as not deployed (older agent)", async () => {
		const { cell } = await renderCells("app_warehouse_scan");
		const station = cell("evt_scan_station");
		expect(station.textContent).toContain("Not on a device you can see");
		expect(byRole("button", "2 unknown", station)).toBeTruthy();
	});

	test("with every Run on a device… blocked, the cell keeps the fact and drops the link", async () => {
		const { cell } = await renderCells("app_invoice_ai", { canDeploy: false });
		expect(cell("evt_gpu_extract").textContent).toContain(
			"Not on a device you can see",
		);
		expect(queryByRole("link", "Run on a device…")).toBeNull();
	});

	test("the column header reads Devices", async () => {
		const { container } = await dom.render(<RunsOnColumnHeader />);
		expect(container.textContent).toBe("Devices");
	});
});

describe("Devices cell: states of the page (APP §4.6)", () => {
	const eligible = "evt_extract_http";
	const cant = "evt_invoice_review";

	test("signed out and token: every cell asks to sign in", async () => {
		for (const status of ["signed_out", "token"] as const) {
			const { text, unmount } = await renderCells("app_invoice_ai", { status });
			expect(text(eligible)).toBe("Sign in to see devices");
			expect(text(cant)).toBe("Sign in to see devices");
			expect(queryByRole("button")).toBeNull();
			await unmount();
		}
	});

	test("hub-off: every cell says device status is off on this hub", async () => {
		const { text } = await renderCells("app_invoice_ai", { status: "hub_off" });
		expect(text(eligible)).toBe("Device status off on this hub");
		expect(text(cant)).toBe("Device status off on this hub");
	});

	test("read-only role without flows: unknown for events that could run, the reason for the others", async () => {
		const { text, cell } = await renderCells("app_invoice_ai", {
			status: "blind",
		});
		expect(text(eligible)).toBe("Unknown: you can't read this app's flows");
		expect(text(cant)).toBe("Can't run on devicesFollows the latest flow");
		expect(queryByRole("button", /Where/, cell(eligible))).toBeNull();
		expect(byRole("button", /^Why Review queue/, cell(cant))).toBeTruthy();
	});

	test("loading and a failed read are never “Not on a device” (R6)", async () => {
		const loading = await renderCells("app_invoice_ai", { status: "loading" });
		expect(loading.text(eligible)).toBe("Checking devices…");
		await loading.unmount();
		const failed = await renderCells("app_invoice_ai", { status: "error" });
		expect(failed.text(eligible)).toBe("Device status unavailable");
		expect(failed.container.textContent).not.toContain("Not on a device");
	});

	test("no machine vocabulary and no primary button in any state (R2, R3)", async () => {
		for (const status of [
			"ready",
			"loading",
			"error",
			"blind",
			"hub_off",
			"signed_out",
		] as const) {
			const { container, unmount } = await renderCells("app_crm_sync", {
				status,
				both: true,
			});
			expect(container.textContent).not.toMatch(MACHINE_WORDS);
			expect(document.querySelectorAll("[data-dv-primary]")).toHaveLength(0);
			await unmount();
		}
	});
});

describe("Devices summary of a phone row (APP §4.7)", () => {
	test("one sentence per event that opens the same popover", async () => {
		const { text, cell } = await renderCells("app_invoice_ai", {
			compact: true,
		});
		expect(text("evt_extract_http")).toBe(
			"On edge-berlin-01 · 1 older · 1 locked",
		);
		expect(text("evt_gpu_extract")).toBe(
			"Not on a device you can see · 1 locked",
		);
		expect(text("evt_invoice_review")).toBe(
			"Can't run on devices: Follows the latest flow",
		);
		await click(
			byRole("button", /^On edge-berlin-01/, cell("evt_extract_http")),
		);
		expect(byRole("dialog", "Where Extract invoice runs")).toBeTruthy();
	});

	test("page states stay plain text", async () => {
		const { text } = await renderCells("app_invoice_ai", {
			compact: true,
			status: "hub_off",
		});
		expect(text("evt_extract_http")).toBe("Device status off on this hub");
		expect(queryByRole("button")).toBeNull();
	});
});

describe("On devices popover (APP §4.4)", () => {
	async function openWhere(
		appId: SampleApp,
		eventId: string,
		options: SampleValueOptions = {},
	) {
		const view = await renderCells(appId, options);
		const name = nameOf(appId, eventId);
		await click(byRole("button", `Where ${name} runs`, view.cell(eventId)));
		return { ...view, dialog: byRole("dialog", `Where ${name} runs`) };
	}

	test("lists each device that serves the event with its service, state, pins and drift", async () => {
		const { dialog } = await openWhere("app_invoice_ai", "evt_extract_http");
		const text = dialog.textContent ?? "";
		expect(text).toContain("Web request · Served by the device");
		expect(text).toContain("Devices run Invoice AI online.");
		expect(text).toContain("Serves it · 1");
		const row = dialog.querySelector<HTMLElement>(
			"[data-on-device='edge-berlin-01']",
		);
		expect(row?.textContent).toContain("Starting");
		expect(row?.textContent).toContain("event 1.4.0 · flow 2.1.0");
		expect(row?.textContent).toContain("newest has event 1.5.0 · flow 2.2.0");
		const service =
			"/library/config/devices?id=app_invoice_ai&device=edge-berlin-01&service=invoice-extractor";
		expect(
			byRole("link", "invoice-extractor", dialog).getAttribute("href"),
		).toBe(service);
		expect(byRole("link", "Open", dialog).getAttribute("href")).toBe(service);
	});

	test("unknown devices come with Unlock…, which asks the overlay host", async () => {
		const { dialog } = await openWhere("app_invoice_ai", "evt_extract_http");
		expect(dialog.textContent).toContain("Status unknown · 1");
		const row = dialog.querySelector<HTMLElement>(
			"[data-on-device='lab-gpu-02']",
		);
		expect(row?.textContent).toContain("Unknown until unlocked");
		await click(byRole("button", "Unlock…", row ?? dialog));
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "unlock",
			deviceId: "lab-gpu-02",
		});
	});

	test("the foot runs it on another device and opens App › Devices by event", async () => {
		const { dialog } = await openWhere("app_invoice_ai", "evt_extract_http");
		expect(
			byRole("link", "Run on another device…", dialog).getAttribute("href"),
		).toBe(
			"/library/config/devices?id=app_invoice_ai&flow=deploy&mode=new&event=evt_extract_http&from=events",
		);
		expect(byRole("link", "Open in Devices", dialog).getAttribute("href")).toBe(
			"/library/config/devices?id=app_invoice_ai&by=event&event=evt_extract_http",
		);
		expect(dialog.textContent).toContain("Doesn't serve it · 3");
		expect(
			allByRole("link", "Deploy here", dialog)[0]?.getAttribute("href"),
		).toBe(
			"/library/config/devices?id=app_invoice_ai&flow=deploy&mode=new&device=studio-mac-mini&event=evt_extract_http&from=events",
		);
	});

	test("an event no device runs yet says why, and the head stamp says locked", async () => {
		const { dialog } = await openWhere("app_invoice_ai", "evt_invoice_mcp");
		expect(dialog.textContent).toContain(
			"New in v1.5.0, so no device runs it yet.",
		);
		expect(byRole("link", "Run on a device…", dialog)).toBeTruthy();
		expect(
			dialog.querySelector("[data-stamp][data-age='locked']"),
		).toBeTruthy();
	});

	test("a device that refused the event shows its own words", async () => {
		const { dialog } = await openWhere("app_crm_sync", "evt_crm_watch");
		const row = dialog.querySelector<HTMLElement>(
			"[data-on-device='edge-berlin-01']",
		);
		expect(row?.textContent).toContain("Can't run here");
		expect(row?.textContent).toContain("requires sandboxed services");
		expect(queryByRole("link", "Deploy here", row ?? dialog)).toBeNull();
	});

	test("a status without an event list explains the interim (older agent)", async () => {
		const { dialog } = await openWhere(
			"app_warehouse_scan",
			"evt_scan_station",
		);
		const row = dialog.querySelector<HTMLElement>(
			"[data-on-device='warehouse-pi']",
		);
		expect(row?.textContent).toContain(
			"Unknown: the status snapshot has no event list",
		);
		expect(row?.textContent).toContain(
			"It runs scanner-ingest, but its status doesn't list the events it serves. An agent update on the device adds them.",
		);
	});

	test("blocked: Run on another device… stays visible, disabled, with the reason, and goes nowhere (R7)", async () => {
		const opened: string[] = [];
		const { dialog } = await openWhere("app_invoice_ai", "evt_extract_http", {
			canDeploy: false,
			opened,
		});
		const run = byRole("button", "Run on another device…", dialog);
		expect(run.getAttribute("aria-disabled")).toBe("true");
		expect(dialog.textContent).toContain(
			"No device can take a deploy right now",
		);
		expect(queryByRole("link", "Deploy here", dialog)).toBeNull();
		await click(run);
		expect(opened).toEqual([]);
	});

	test("one request opens one popover, also with both variants of the cell mounted", async () => {
		const value: Partial<EventsDevicesValue> = {
			focus: { eventId: "evt_extract_http", token: 1 },
		};
		await renderCells("app_invoice_ai", { both: true, value });
		expect(allByRole("dialog")).toHaveLength(1);
	});

	test("a long group shows five devices and offers the rest (R11)", async () => {
		const [, , studio] = sampleDevices();
		const extra = Array.from({ length: 5 }, (_, index) => ({
			...studio,
			id: `spare-${index}`,
			name: `spare-${index}`,
			services: [],
		}));
		const { dialog } = await openWhere("app_invoice_ai", "evt_extract_http", {
			devices: [...sampleDevices(), ...extra],
		});
		const rows = () =>
			dialog.querySelectorAll("[data-on-device][data-state]").length;
		expect(dialog.textContent).toContain("Doesn't serve it · 8");
		expect(rows()).toBe(5);
		await click(byRole("button", "Show 3 more", dialog));
		expect(rows()).toBe(8);
		expect(queryByRole("button", /^Show \d+ more$/, dialog)).toBeNull();
	});
});

describe("Why an event can't run on devices (APP §7.3)", () => {
	async function openWhy(
		appId: SampleApp,
		eventId: string,
		options: SampleValueOptions = {},
	) {
		const view = await renderCells(appId, options);
		const label = `Why ${nameOf(appId, eventId)} can't run on devices`;
		await click(byRole("button", label, view.cell(eventId)));
		return { ...view, dialog: byRole("dialog", label) };
	}

	test("gives the long reason, the fix and what it doesn't affect", async () => {
		const openedEvents: string[] = [];
		const { dialog } = await openWhy("app_crm_sync", "evt_crm_rest", {
			value: { openEvent: (eventId) => openedEvents.push(eventId) },
		});
		const text = dialog.textContent ?? "";
		expect(text).toContain(
			"Splits traffic with a canary. A device can't split traffic; end the canary first.",
		);
		expect(text).toContain(
			"To fix it: End the canary in the event's Canary section.",
		);
		expect(text).toContain("3 events can run on a device.");
		expect(text).not.toMatch(MACHINE_WORDS);
		await click(byRole("button", "Open the event…", dialog));
		expect(openedEvents).toEqual(["evt_crm_rest"]);
		expect(queryByRole("dialog")).toBeNull();
	});

	test("a paused event points at the play button in its row", async () => {
		const { dialog } = await openWhy(
			"app_support_portal",
			"evt_support_mailbox",
		);
		expect(dialog.textContent).toContain("Paused, so it can't run on devices");
		expect(dialog.textContent).toContain(
			"Activate it with the play button in its row",
		);
		expect(queryByRole("button", "Open the event…", dialog)).toBeNull();
	});

	test("a type that can't run has no fix to offer", async () => {
		const { dialog } = await openWhy("app_crm_sync", "evt_crm_hourly");
		expect(dialog.textContent).toContain(
			"Schedules can't run on a device yet.",
		);
		expect(dialog.textContent).not.toContain("To fix it:");
		expect(byRole("link", "Open in Devices", dialog).getAttribute("href")).toBe(
			"/library/config/devices?id=app_crm_sync&by=event&event=evt_crm_hourly",
		);
	});

	test("without device data the reason still shows, without a link into Devices", async () => {
		const { dialog } = await openWhy("app_crm_sync", "evt_crm_hourly", {
			status: "blind",
		});
		expect(dialog.textContent).toContain(
			"Schedules can't run on a device yet.",
		);
		expect(queryByRole("link", "Open in Devices", dialog)).toBeNull();
	});
});
