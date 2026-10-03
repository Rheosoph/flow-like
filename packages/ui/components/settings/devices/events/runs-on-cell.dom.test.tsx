import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	APPS,
	HASH,
	NOW0,
	SERVICES,
	sampleDevices,
	svc,
	v,
} from "../../../../lib/device-management/model/__fixtures__/apps";
import type {
	AppScheduleRow,
	ServiceSchedule,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import {
	allByRole,
	byRole,
	click,
	installDom,
	queryByRole,
	settle,
} from "../testing/dom-harness";
import type { EventsDevicesValue } from "./events-devices";
import type { SampleApp, SampleValueOptions } from "./events-test-kit";

const dom = installDom();
const { RunsOnCell, RunsOnColumnHeader, runsOnReasonId } = await import(
	"./runs-on-cell"
);
const {
	SampleEventsDevices,
	sampleAppWith: withEvent,
	sampleEvents,
	sampleValue,
} = await import("./events-test-kit");
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
		<SampleEventsDevices value={value} now={options.now}>
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

const has = (root: ParentNode, selector: string) =>
	root.querySelector(selector) !== null;

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
		expect(text("evt_crm_rest")).toBe(
			"Can't run on devicesSplits traffic with a canary",
		);
		expect(queryByRole("link", "Run on a device…", cell("evt_crm_rest"))).toBe(
			null,
		);
	});

	test("a schedule is an event like any other: where it runs, and Run on a device…", async () => {
		const { cell } = await renderCells("app_crm_sync");
		const hourly = cell("evt_crm_hourly");
		expect(hourly.getAttribute("data-runs-on")).toBe("row");
		expect(firstLine(hourly)).toContain("Not on a device you can see");
		expect(
			byRole("link", "Run on a device…", hourly).getAttribute("href"),
		).toBe(
			"/library/config/devices?id=app_crm_sync&flow=deploy&mode=new&event=evt_crm_hourly&from=events",
		);
	});

	test("local-only: both served events show the device; a paused one says to activate it", async () => {
		const { cell, text } = await renderCells("app_support_portal");
		expect(
			byRole("link", "edge-berlin-01", cell("evt_support_chat")),
		).toBeTruthy();
		expect(
			byRole("link", "edge-berlin-01", cell("evt_support_http")),
		).toBeTruthy();
		// A quick action is a person-started event now: a row like any other.
		expect(cell("evt_support_reply").getAttribute("data-runs-on")).toBe("row");
		expect(text("evt_support_reply")).toBe(
			"Not on a device you can see1 lockedRun on a device…",
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
	const cant = "evt_invoice_inbox";

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
		expect(text(cant)).toBe("Can't run on devicesHandled by the hub");
		expect(queryByRole("button", /Where/, cell(eligible))).toBeNull();
		expect(byRole("button", /^Why Invoice mailbox/, cell(cant))).toBeTruthy();
		for (const eventId of ["evt_invoice_review", "evt_invoice_reconcile"])
			expect(text(eventId)).toBe("Unknown: you can't read this app's flows");
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
		expect(text("evt_invoice_inbox")).toBe(
			"Can't run on devices: Handled by the hub",
		);
		for (const eventId of ["evt_invoice_review", "evt_invoice_reconcile"])
			expect(text(eventId)).toBe("Not on a device you can see · 1 locked");
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
		expect(text).toContain("Endpoint · POST /extract");
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

const RECONCILE = "evt_invoice_reconcile";

/** The device's own facts about the nightly schedule, as a live status carries them. */
const RECONCILE_ENTRY: ServiceSchedule = {
	event_id: RECONCILE,
	expression: "0 0 2 * * *",
	timezone: "Europe/Berlin",
	hold: null,
	next_at: NOW0 + 12 * 3600,
	last_at: NOW0 - 12 * 3600,
	last_outcome: "succeeded",
	runs: 3,
	failed: 0,
	skipped: 0,
};

function reconcileService(patch: Partial<ServiceView> = {}): ServiceView {
	return svc({
		deviceId: "edge-berlin-01",
		serviceId: "invoice-reconcile",
		projectId: "app_invoice_ai",
		source: "online",
		events: [
			{
				event_id: RECONCILE,
				event_version: v("1.0.0"),
				board_version: v("1.3.0"),
			},
		],
		appVersion: { label: "v1.5.0", hash: HASH.invoice15 },
		schedules: [RECONCILE_ENTRY],
		...patch,
	});
}

/** The sample fleet with a service on edge-berlin-01 that serves the nightly schedule. */
const withReconcile = (patch: Partial<ServiceView> = {}) =>
	sampleDevices().map((device) =>
		device.id === "edge-berlin-01" && Array.isArray(device.services)
			? { ...device, services: [...device.services, reconcileService(patch)] }
			: device,
	);

const onEdge = (
	state: "device" | "released",
	placement = "invoice-reconcile",
): AppScheduleRow =>
	state === "device"
		? {
				event_id: RECONCILE,
				state,
				since: NOW0 - 86_400,
				seen_at: NOW0 - 600,
				device_id: "edge-berlin-01",
				placement_id: placement,
			}
		: {
				event_id: RECONCILE,
				state,
				since: NOW0 - 60,
				device_id: "edge-berlin-01",
				placement_id: placement,
			};

describe("A schedule on the Events list: when it runs and where", () => {
	async function openSchedule(options: SampleValueOptions = {}) {
		const view = await renderCells("app_invoice_ai", { now: NOW0, ...options });
		const cell = view.cell(RECONCILE);
		await click(byRole("button", "Where Nightly reconciliation runs", cell));
		const dialog = byRole("dialog", "Where Nightly reconciliation runs");
		const where = () =>
			dialog.querySelector<HTMLElement>("[data-schedule-where]");
		return { ...view, cell, dialog, where };
	}

	test("the hub runs it: the popover says when and where, and offers a device", async () => {
		const { cell, dialog, where } = await openSchedule();
		expect(firstLine(cell)).toContain("Not on a device you can see");
		expect(has(cell, "[data-runs-on-where]")).toBe(false);
		expect(dialog.textContent).toContain(
			"Schedule · At 02:00 every day · Europe/Berlin",
		);
		expect(where()?.getAttribute("data-schedule-where")).toBe("hub");
		expect(where()?.textContent).toBe("Where it runsRuns on the hub.");
		expect(queryByRole("button", "Run it on the hub again", dialog)).toBeNull();
		expect(byRole("link", "Run on a device…", dialog)).toBeTruthy();
	});

	test("a local-only app has no hub: the popover says where a schedule runs today", async () => {
		const { cell } = await renderCells("app_crm_sync", { now: NOW0 });
		await click(
			byRole("button", "Where Hourly sync runs", cell("evt_crm_hourly")),
		);
		const dialog = byRole("dialog", "Where Hourly sync runs");
		expect(dialog.textContent).toContain(
			"Schedule · At :00 past every hour · UTC (the event sets no time zone)",
		);
		expect(
			dialog.querySelector("[data-schedule-where='local']")?.textContent,
		).toBe(
			"Runs in the desktop app while it is open. On a device it runs without a computer.",
		);
		expect(queryByRole("button", "Run it on the hub again", dialog)).toBeNull();
	});

	test("released to a service that has not started it: the hub still runs it", async () => {
		const givenBack: string[] = [];
		const { cell, dialog, where } = await openSchedule({
			schedules: [onEdge("released", "invoice-extractor")],
			givenBack,
		});
		const sentence =
			"Moves to edge-berlin-01 › invoice-extractor when that service starts it. The hub runs it until then.";
		expect(
			cell.querySelector("[data-runs-on-where='released']")?.textContent,
		).toBe(sentence);
		expect(where()?.textContent).toContain(sentence);
		expect(byRole("link", "Run on a device…", dialog)).toBeTruthy();
		await click(byRole("button", "Run it on the hub again", dialog));
		const confirm = byRole("region", "Run it on the hub again", dialog);
		expect(confirm.textContent).toContain(
			"The hub keeps running Nightly reconciliation. edge-berlin-01 can no longer take it over.",
		);
		await click(byRole("button", "Run it on the hub again", confirm));
		await settle();
		expect(givenBack).toEqual([RECONCILE]);
		expect(where()?.textContent).toContain(
			"The hub runs Nightly reconciliation again from 12:05.",
		);
	});

	test("a running service is known to run it: Runs on the device, with the device's next and last run", async () => {
		const { cell, dialog, where } = await openSchedule({
			devices: withReconcile(),
			schedules: [onEdge("device")],
		});
		expect(byRole("link", "edge-berlin-01", cell)).toBeTruthy();
		expect(cell.querySelector("[data-runs-on-next]")?.textContent).toBe(
			"Next run Oct 1 at 02:00 GMT+2 · in 12 hr. · Oct 1 at 00:00 your time",
		);
		expect(has(cell, "[data-runs-on-where]")).toBe(false);
		expect(where()?.getAttribute("data-schedule-where")).toBe("device");
		expect(where()?.textContent).toContain(
			"Runs on edge-berlin-01, not on the hub.",
		);
		const runs = [...dialog.querySelectorAll("[data-schedule-run]")].map(
			(line) => line.textContent,
		);
		expect(runs).toEqual([
			"Next run Oct 1 at 02:00 GMT+2 · in 12 hr. · Oct 1 at 00:00 your time",
			"Last run 02:00 GMT+2 · 12 hr. ago · 00:00 your time · succeeded",
		]);
		const run = byRole("button", "Run on another device…", dialog);
		expect(run.getAttribute("aria-disabled")).toBe("true");
		expect(dialog.textContent).toContain(
			"A schedule runs in one place. Run it on the hub again first.",
		);
	});

	test("Run it on the hub again asks first, says what happens, and then says when the hub takes over", async () => {
		const givenBack: string[] = [];
		const { dialog, where } = await openSchedule({
			devices: withReconcile(),
			schedules: [onEdge("device")],
			givenBack,
		});
		await click(byRole("button", "Run it on the hub again", dialog));
		expect(givenBack).toEqual([]);
		const confirm = byRole("region", "Run it on the hub again", dialog);
		expect(confirm.textContent).toContain(
			"The hub runs Nightly reconciliation again in about 5 minutes, or in about an hour while edge-berlin-01 is still running its service. edge-berlin-01 stops running it at its next check, within 30 minutes.",
		);
		expect(confirm.textContent).toContain(
			"Runs that are due before the hub takes over are skipped, never run twice.",
		);
		await click(byRole("button", "Run it on the hub again", confirm));
		await settle();
		expect(givenBack).toEqual([RECONCILE]);
		expect(where()?.textContent).toContain(
			"The hub runs Nightly reconciliation again from 12:05.",
		);
	});

	test("a person who can't edit the app's events sees the way back, disabled, with the reason (R7)", async () => {
		const givenBack: string[] = [];
		const { dialog } = await openSchedule({
			devices: withReconcile(),
			schedules: [onEdge("device")],
			canEditEvents: false,
			givenBack,
		});
		const back = byRole("button", "Run it on the hub again", dialog);
		expect(back.getAttribute("aria-disabled")).toBe("true");
		expect(dialog.textContent).toContain(
			"Only someone who can edit this app's events can move it.",
		);
		await click(back);
		expect(queryByRole("region", "Run it on the hub again", dialog)).toBeNull();
		expect(givenBack).toEqual([]);
	});

	test("assigned to a stopped or failing service: never “Runs on”, and the cell says so", async () => {
		const stopped = await openSchedule({
			devices: withReconcile({
				desired: "stopped",
				observed: "stopped",
				conv: "stopped_by_user",
			}),
			schedules: [onEdge("device")],
		});
		const sentence =
			"Assigned to edge-berlin-01 › invoice-reconcile, but nothing runs it: the service is stopped.";
		expect(
			stopped.cell.querySelector("[data-runs-on-where='device_idle']")
				?.textContent,
		).toBe(sentence);
		expect(stopped.where()?.textContent).toContain(sentence);
		expect(stopped.dialog.textContent).not.toContain("Runs on edge-berlin-01");
		expect(has(stopped.cell, "[data-runs-on-next]")).toBe(false);
		expect(
			[...stopped.dialog.querySelectorAll("[data-schedule-run]")].map(
				(line) => line.textContent,
			),
		).toEqual(["No runs while the service is not running."]);
		await stopped.unmount();
		const failing = await openSchedule({
			devices: withReconcile({ observed: "restarting", conv: "crash_looping" }),
			schedules: [onEdge("device")],
		});
		expect(failing.where()?.textContent).toContain(
			"Assigned to edge-berlin-01 › invoice-reconcile, but nothing runs it: the service keeps failing.",
		);
	});

	test("assigned to a service that waits for the hub, or that was removed", async () => {
		const held = await openSchedule({
			devices: withReconcile({
				schedules: [{ ...RECONCILE_ENTRY, hold: "hub_unreachable" }],
			}),
			schedules: [onEdge("device")],
		});
		expect(held.where()?.textContent).toContain(
			"Assigned to edge-berlin-01 › invoice-reconcile, but nothing runs it: the service is waiting for the hub to confirm it.",
		);
		expect(
			[...held.dialog.querySelectorAll("[data-schedule-run]")].map(
				(line) => line.textContent,
			),
		).toEqual(["Waiting for the hub to confirm that it stopped running it."]);
		await held.unmount();
		const removed = await openSchedule({
			schedules: [onEdge("device", "invoice-old")],
		});
		expect(removed.where()?.textContent).toContain(
			"Assigned to edge-berlin-01 › invoice-old, but nothing runs it: the service was removed.",
		);
		expect(
			byRole("button", "Run it on the hub again", removed.dialog),
		).toBeTruthy();
	});

	test("assigned to a device that can't be read: unknown stays unknown", async () => {
		const locked: AppScheduleRow = {
			event_id: RECONCILE,
			state: "device",
			since: NOW0 - 86_400,
			seen_at: NOW0 - 1200,
			device_id: "lab-gpu-02",
			placement_id: "invoice-extractor-gpu",
		};
		const recent = await openSchedule({ schedules: [locked] });
		const sentence =
			"Assigned to lab-gpu-02. It last confirmed 20 min. ago; unlock the device to see whether it runs.";
		expect(
			recent.cell.querySelector("[data-runs-on-where='device_unconfirmed']")
				?.textContent,
		).toBe(sentence);
		expect(recent.dialog.textContent).not.toContain("Runs on lab-gpu-02");
		await recent.unmount();
		const stale = await openSchedule({
			schedules: [{ ...locked, seen_at: NOW0 - 3 * 3600 }],
		});
		expect(stale.where()?.textContent).toContain(
			"Assigned to lab-gpu-02, which has not confirmed since 09:00. It may be off.",
		);
		await stale.unmount();
		const hidden = await openSchedule({
			schedules: [
				{
					event_id: RECONCILE,
					state: "device",
					since: NOW0 - 86_400,
					seen_at: NOW0 - 1200,
				},
			],
		});
		expect(hidden.where()?.textContent).toContain(
			"Assigned to a device you can't see, not the hub. It last confirmed 20 min. ago.",
		);
	});

	test("on its way back: the hub runs it again at a time, and nothing is offered meanwhile", async () => {
		const { cell, dialog, where } = await openSchedule({
			schedules: [
				{ event_id: RECONCILE, state: "returning", hub_resumes_at: NOW0 + 300 },
			],
		});
		expect(
			cell.querySelector("[data-runs-on-where='returning']")?.textContent,
		).toBe("Returns to the hub at 12:05.");
		expect(where()?.textContent).toContain("Returns to the hub at 12:05.");
		expect(queryByRole("button", "Run it on the hub again", dialog)).toBeNull();
	});

	test("a status snapshot computes the next run from the schedule and says so", async () => {
		const { cell } = await openSchedule({
			devices: withReconcile({
				freshness: { src: "snap", age: "current", at: NOW0 - 40 },
				schedules: [
					{
						event_id: RECONCILE,
						expression: "0 0 2 * * *",
						timezone: "Europe/Berlin",
						hold: null,
						last_outcome: "failed",
					},
				],
			}),
			schedules: [onEdge("device")],
		});
		expect(cell.querySelector("[data-runs-on-next]")?.textContent).toBe(
			"Next run by its schedule: Oct 1 at 02:00 GMT+2 · in 12 hr. · Oct 1 at 00:00 your time",
		);
	});

	test("an older agent can't take a schedule: it says so and links to the agent update", async () => {
		const devices = sampleDevices().map((device) =>
			device.id === "studio-mac-mini" ? { ...device, features: {} } : device,
		);
		const { dialog } = await openSchedule({ devices });
		const row = dialog.querySelector<HTMLElement>(
			"[data-on-device='studio-mac-mini']",
		);
		expect(row?.textContent).toContain(
			"studio-mac-mini's agent is too old to run schedules.",
		);
		expect(
			byRole(
				"link",
				"Update the device agent to run schedules",
				row ?? dialog,
			).getAttribute("href"),
		).toBe(
			"/library/config/devices?id=app_invoice_ai&device=studio-mac-mini&tab=settings",
		);
		expect(queryByRole("link", "Deploy here", row ?? dialog)).toBeNull();
	});
});

describe("An event that follows Latest on the Events list", () => {
	const REVIEW = "evt_invoice_review";

	function reviewService(pin: [string, string]): ServiceView {
		return svc({
			deviceId: "studio-mac-mini",
			serviceId: "invoice-review",
			projectId: "app_invoice_ai",
			source: "online",
			events: [
				{
					event_id: REVIEW,
					event_version: v(pin[0]),
					board_version: v(pin[1]),
				},
			],
			appVersion: { label: "v1.5.0", hash: HASH.invoice15 },
		});
	}

	const withReview = (pin: [string, string]) =>
		sampleDevices().map((device) =>
			device.id === "studio-mac-mini" && Array.isArray(device.services)
				? { ...device, services: [...device.services, reviewService(pin)] }
				: device,
		);

	async function openReview(options: SampleValueOptions = {}) {
		const view = await renderCells("app_invoice_ai", options);
		await click(byRole("button", "Where Review queue runs", view.cell(REVIEW)));
		const dialog = byRole("dialog", "Where Review queue runs");
		const row = dialog.querySelector<HTMLElement>(
			"[data-on-device='studio-mac-mini']",
		);
		return { ...view, dialog, row };
	}

	test("it can run on a device, like an event with a pinned flow", async () => {
		const { cell } = await renderCells("app_invoice_ai");
		const review = cell(REVIEW);
		expect(review.getAttribute("data-runs-on")).toBe("row");
		expect(byRole("link", "Run on a device…", review)).toBeTruthy();
	});

	test("a device that runs the flow's current version reads newest, tagged Follows Latest, never “Latest”", async () => {
		const { row } = await openReview({
			devices: withReview(["0.9.0", "0.9.2"]),
		});
		expect(row?.textContent).toContain("event 0.9.0 · flow 0.9.2");
		expect(row?.textContent).toContain("newest");
		expect(row?.querySelector("[data-follows-latest]")?.textContent).toBe(
			"Follows Latest",
		);
		expect(row?.textContent).not.toContain("flow Latest");
	});

	test("the flow has edits that no version holds: never “newest”", async () => {
		const { row, cell } = await openReview({
			devices: withReview(["0.9.0", "0.9.2"]),
			app: withEvent("app_invoice_ai", REVIEW, {
				flow: { current: null, newest: v("0.9.2") },
			}),
		});
		expect(row?.textContent).toContain("Newer flow edits are available");
		expect(row?.textContent).not.toContain("newest");
		expect(
			byRole("link", "studio-mac-mini", cell(REVIEW)).getAttribute("title"),
		).toBe("invoice-review · Running · newer flow edits are available");
		// The device runs the newest version there is; it is behind the edits, not on an older version.
		expect(
			cell(REVIEW).querySelector("[data-runs-on-edits]")?.textContent,
		).toBe("1 runs the flow from before its current edits");
		expect(cell(REVIEW).textContent).not.toContain("older version");
	});

	test("the flow's state is not known: unknown, never “newest”", async () => {
		const { flow: _flow, ...unread } = APPS.app_invoice_ai.events[0];
		const { row } = await openReview({
			devices: withReview(["0.9.0", "0.9.2"]),
			app: {
				...APPS.app_invoice_ai,
				events: [unread, ...APPS.app_invoice_ai.events.slice(1)],
			},
		});
		expect(row?.textContent).not.toContain("newest");
		expect(row?.textContent).toContain(
			"Unknown: the flow's current version isn't read yet",
		);
	});

	test("a device on an older flow version says what the newest has", async () => {
		const { row } = await openReview({
			devices: withReview(["0.9.0", "0.9.1"]),
		});
		expect(row?.textContent).toContain("newest has flow 0.9.2");
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
		expect(text).toContain("4 events can run on a device.");
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
		const { dialog } = await openWhy("app_warehouse_scan", "evt_scan_deeplink");
		expect(dialog.textContent).toContain(
			"Deep links and location regions only work in the desktop app.",
		);
		expect(dialog.textContent).not.toContain("To fix it:");
		expect(queryByRole("button", "Open the event…", dialog)).toBeNull();
		expect(byRole("link", "Open in Devices", dialog).getAttribute("href")).toBe(
			"/library/config/devices?id=app_warehouse_scan&by=event&event=evt_scan_deeplink",
		);
	});

	test("without device data the reason still shows, without a link into Devices", async () => {
		const { dialog } = await openWhy("app_partner_reports", "evt_report_bot", {
			status: "blind",
		});
		expect(dialog.textContent).toContain("Teams bots run on the hub.");
		expect(queryByRole("link", "Open in Devices", dialog)).toBeNull();
	});

	test("a schedule that a device can't read says what is wrong and opens the event", async () => {
		const openedEvents: string[] = [];
		const { dialog, cell } = await openWhy("app_crm_sync", "evt_crm_hourly", {
			app: withEvent("app_crm_sync", "evt_crm_hourly", {
				schedule: { expression: "*/30 * * * * *" },
			}),
			value: { openEvent: (eventId) => openedEvents.push(eventId) },
		});
		expect(cell("evt_crm_hourly").textContent).toBe(
			"Can't run on devicesMore than once a minute",
		);
		expect(dialog.textContent).toContain(
			"Runs more often than once a minute. Devices run a schedule at most once a minute.",
		);
		await click(byRole("button", "Open the event…", dialog));
		expect(openedEvents).toEqual(["evt_crm_hourly"]);
	});

	test("a schedule without an expression says so; a one-time one can run, unless its date can't be read", async () => {
		const missing = await renderCells("app_crm_sync", {
			app: withEvent("app_crm_sync", "evt_crm_hourly", { schedule: {} }),
		});
		expect(missing.text("evt_crm_hourly")).toBe(
			"Can't run on devicesNo schedule set",
		);
		await missing.unmount();
		const once = await renderCells("app_crm_sync", {
			app: withEvent("app_crm_sync", "evt_crm_hourly", {
				schedule: { scheduled_for: { date: "2026-12-01", time: "08:00" } },
			}),
		});
		expect(once.cell("evt_crm_hourly").getAttribute("data-runs-on")).toBe(
			"row",
		);
		expect(once.text("evt_crm_hourly")).toBe(
			"Not on a device you can see1 lockedRun on a device…",
		);
		await once.unmount();
		const unreadable = await renderCells("app_crm_sync", {
			app: withEvent("app_crm_sync", "evt_crm_hourly", {
				schedule: { scheduled_for: { date: "2026-02-30", time: "08:00" } },
			}),
		});
		expect(unreadable.text("evt_crm_hourly")).toBe(
			"Can't run on devicesSchedule can't be read",
		);
	});

	test("a paused event a device still runs says so and links to its service", async () => {
		const { dialog, cell } = await openWhy(
			"app_support_portal",
			"evt_support_http",
			{
				app: withEvent("app_support_portal", "evt_support_http", {
					active: false,
				}),
			},
		);
		expect(cell("evt_support_http").textContent).toBe(
			"Paused: activate it firstedge-berlin-01 still runs it",
		);
		const still = dialog.querySelector<HTMLElement>(
			"[data-still-served='edge-berlin-01']",
		);
		expect(still?.textContent).toContain(
			"Paused in Events. edge-berlin-01 still runs it until you stop or update support-bot.",
		);
		expect(
			byRole("link", "Open support-bot", still ?? dialog).getAttribute("href"),
		).toBe(
			"/library/config/devices?id=app_support_portal&device=edge-berlin-01&service=support-bot",
		);
	});
});
