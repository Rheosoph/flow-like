import { afterAll, afterEach, describe, expect, test } from "bun:test";
import {
	APPS,
	HASH,
	ME,
	NOW0,
	SHOP_ONCE_AT,
	sampleDevices,
	svc,
	v,
} from "../../../../lib/device-management/model/__fixtures__/apps";
import type {
	AppDevicePlacements,
	AppScheduleRow,
	GateFailure,
	ServiceBot,
	ServiceSchedule,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import {
	byRole,
	click,
	installDom,
	queryByRole,
	settle,
} from "../testing/dom-harness";
import type { SampleApp, SampleValueOptions } from "./events-test-kit";

const dom = installDom();
const { RunsOnCell, runsOnReasonId } = await import("./runs-on-cell");
const { SampleEventsDevices, sampleEvents, sampleValue } = await import(
	"./events-test-kit"
);
const { useOverlayStore } = await import("../workspace/overlay-store");

afterEach(async () => {
	useOverlayStore.getState().close();
	await dom.cleanup();
});
afterAll(dom.restore);

/* Round two on the Events list (design R2 §6.4 "Events column"): every new kind is a runnable row with its own words. */

const SHOP: SampleApp = "app_shop_assistant";
const EDGE = "edge-berlin-01";
const SERVICE = "shop-assistant";
const ORDERS = "evt_shop_orders";
const FORM = "evt_shop_return";
const TELEGRAM = "evt_shop_telegram";
const DISCORD = "evt_shop_discord";
const PRICES = "evt_shop_prices";

const MACHINE_WORDS = /\b[a-z]+_[a-z_]+\b|\bG\d{1,2}\b/;

const nameOf = (appId: SampleApp, eventId: string) =>
	APPS[appId].events.find((event) => event.id === eventId)?.name ?? "";

async function renderCells(appId: SampleApp, options: SampleValueOptions = {}) {
	const value = sampleValue(appId, { now: NOW0, ...options });
	const view = await dom.render(
		<SampleEventsDevices value={value} now={options.now ?? NOW0}>
			<ul>
				{sampleEvents(appId).map((event) => (
					<li key={event.id}>
						<RunsOnCell appId={appId} eventId={event.id} />
					</li>
				))}
			</ul>
		</SampleEventsDevices>,
	);
	const cell = (eventId: string) => {
		const found = view.container.querySelector<HTMLElement>(
			`[id="${runsOnReasonId(eventId)}"]`,
		);
		if (!found) throw new Error(`no cell for ${eventId}`);
		return found;
	};
	return { ...view, value, cell };
}

async function openWhere(
	appId: SampleApp,
	eventId: string,
	options: SampleValueOptions = {},
) {
	const view = await renderCells(appId, options);
	const name = nameOf(appId, eventId);
	const cell = view.cell(eventId);
	await click(byRole("button", `Where ${name} runs`, cell));
	const dialog = byRole("dialog", `Where ${name} runs`);
	const where = () =>
		dialog.querySelector<HTMLElement>("[data-schedule-where]");
	const served = () =>
		dialog.querySelector<HTMLElement>(`[data-on-device="${EDGE}"]`);
	return { ...view, cell, dialog, where, served };
}

const pin = (eventId: string) => ({
	event_id: eventId,
	event_version: v("1.0.0"),
	board_version: v("1.2.0"),
});

const HELPER: ServiceBot = {
	event_id: TELEGRAM,
	provider: "telegram",
	state: "connected",
	hold: null,
	bot_name: "helper_bot",
	connected_at: NOW0 - 3_600,
	runs: 4,
	runs_today: 2,
	failed: 0,
	dropped: 0,
};

const SUPPORT_BOT: ServiceBot = {
	event_id: DISCORD,
	provider: "discord",
	state: "connected",
	hold: null,
	bot_name: null,
};

const PENDING: ServiceSchedule = {
	event_id: PRICES,
	once_at: SHOP_ONCE_AT,
	once_state: "pending",
	timezone: "Europe/Berlin",
	hold: null,
	next_at: SHOP_ONCE_AT,
	running: false,
	last_at: null,
	last_outcome: null,
};

/** Shop Assistant on edge-berlin-01: the Endpoint, the form, both bots and the one-time schedule. */
function shopService(patch: Partial<ServiceView> = {}): ServiceView {
	return svc({
		deviceId: EDGE,
		serviceId: SERVICE,
		projectId: APPS[SHOP].id,
		source: "online",
		events: [ORDERS, FORM, TELEGRAM, DISCORD, PRICES].map(pin),
		appVersion: { label: "v1.0.0", hash: HASH.shop10 },
		schedules: [PENDING],
		bots: [HELPER, SUPPORT_BOT],
		actions: [{ event_id: FORM, kind: "form", fields: 3, file_fields: 1 }],
		...patch,
	});
}

const withShop = (patch: Partial<ServiceView> = {}) =>
	sampleDevices().map((device) =>
		device.id === EDGE && Array.isArray(device.services)
			? { ...device, services: [...device.services, shopService(patch)] }
			: device,
	);

const claimed = (eventId: string): AppScheduleRow => ({
	event_id: eventId,
	state: "device",
	since: NOW0 - 86_400,
	seen_at: NOW0 - 600,
	device_id: EDGE,
	placement_id: SERVICE,
});

const ACCESS_ENDS = NOW0 + 29 * 86_400;

function approval(
	status: "active" | "revoked" = "active",
): AppDevicePlacements["placements"][number] {
	return {
		device_id: EDGE,
		placement_id: SERVICE,
		deployment_id: `dep-${SERVICE}`,
		relationship: "owner",
		grant: {
			grant_id: "grant-shop",
			status,
			expires_at: ACCESS_ENDS,
			effective_expires_at: status === "revoked" ? NOW0 - 3_600 : ACCESS_ENDS,
			effective_limit: "approval",
			online_access: "read_write",
			model_ids: [],
			max_instances: 1,
			approved_by_user_id: ME,
			created_at: NOW0 - 86_400,
		},
		billing: null,
		instances: { active: 1, newest_lease_expires_at: NOW0 + 600 },
	};
}

const runLines = (root: ParentNode | null) =>
	[...(root?.querySelectorAll("[data-schedule-run]") ?? [])].map(
		(line) => line.textContent,
	);

describe("every new kind on the Events list", () => {
	test("is a runnable row; its popover says the kind in its own words", async () => {
		const heads: Record<string, string> = {
			[ORDERS]: "Endpoint · GET /orders",
			[FORM]: "Form · started by a person",
			[TELEGRAM]:
				"Telegram bot · Runs on its own · stays connected to Telegram",
			[DISCORD]: "Discord bot · Runs on its own · stays connected to Discord",
			[PRICES]: "Schedule · Once on 2026-10-15 at 09:00 · Europe/Berlin",
		};
		for (const [eventId, head] of Object.entries(heads)) {
			const { cell, dialog, unmount } = await openWhere(SHOP, eventId);
			expect([eventId, cell.getAttribute("data-runs-on")]).toEqual([
				eventId,
				"row",
			]);
			expect([eventId, dialog.querySelector("h3 + p")?.textContent]).toEqual([
				eventId,
				head,
			]);
			expect(dialog.textContent).not.toMatch(MACHINE_WORDS);
			await unmount();
		}
		const { cell } = await renderCells(SHOP);
		expect(cell("evt_shop_mail").getAttribute("data-runs-on")).toBe("cant");
	});

	test("a quick action says it is started by a person", async () => {
		const { dialog } = await openWhere(
			"app_support_portal",
			"evt_support_reply",
		);
		expect(dialog.querySelector("h3 + p")?.textContent).toBe(
			"Quick action · started by a person",
		);
	});
});

describe("A bot on the Events list: where it runs, and taking it back", () => {
	test("no device runs it: nothing to take back, and Run on a device… is offered", async () => {
		const { where, dialog } = await openWhere(SHOP, TELEGRAM);
		expect(where()?.getAttribute("data-where-kind")).toBe("bot");
		expect(where()?.textContent).toBe("Where it runsNo device runs it.");
		expect(queryByRole("button", "Take it back", dialog)).toBeNull();
		expect(queryByRole("button", "Run it on the hub again", dialog)).toBeNull();
		expect(byRole("link", "Run on a device…", dialog)).toBeTruthy();
	});

	test("connected from a device: the device and its state, and another device can't take it", async () => {
		const { cell, where, served, dialog } = await openWhere(SHOP, TELEGRAM, {
			devices: withShop(),
			schedules: [claimed(TELEGRAM)],
			grants: [approval()],
		});
		expect(byRole("link", EDGE, cell)).toBeTruthy();
		expect(where()?.textContent).toBe(
			"Where it runsConnected from edge-berlin-01.Take it back",
		);
		expect(runLines(served())).toEqual(["Connected as helper_bot"]);
		expect(queryByRole("button", "Run it on the hub again", dialog)).toBeNull();
		const another = byRole("button", "Run on another device…", dialog);
		expect(another.getAttribute("aria-disabled")).toBe("true");
		expect(dialog.textContent).toContain(
			"A bot runs in one place. Take it back first.",
		);
	});

	test("Take it back asks first and says that the token stays on the device", async () => {
		const givenBack: string[] = [];
		const { where, dialog } = await openWhere(SHOP, TELEGRAM, {
			devices: withShop(),
			schedules: [claimed(TELEGRAM)],
			grants: [approval()],
			givenBack,
		});
		await click(byRole("button", "Take it back", dialog));
		expect(givenBack).toEqual([]);
		const confirm = byRole("region", "Take it back", dialog);
		const text = confirm.textContent ?? "";
		expect(text).toContain(
			"edge-berlin-01 disconnects it at its next check, within 30 minutes. Nothing else runs it afterwards.",
		);
		expect(text).toContain(
			"People who message Shop helper get no answer once it disconnects.",
		);
		expect(text).toMatch(
			/edge-berlin-01 disconnects Shop helper at its next check with the hub, within 30 minutes\. If it can't reach the hub, it can stay connected until its cloud access ends on [^.]+\. The bot token stays on edge-berlin-01: to cut the bot off for certain, replace the token with BotFather\./,
		);
		expect(text).not.toMatch(MACHINE_WORDS);
		await click(byRole("button", "Take it back", confirm));
		await settle();
		expect(givenBack).toEqual([TELEGRAM]);
		expect(where()?.textContent).toContain(
			"Taken back. edge-berlin-01 disconnects it within 30 minutes; nothing runs it afterwards.",
		);
	});

	test("a Discord bot names the Developer Portal; without a visible approval the sentence has no date", async () => {
		const { dialog } = await openWhere(SHOP, DISCORD, {
			devices: withShop(),
			schedules: [claimed(DISCORD)],
		});
		await click(byRole("button", "Take it back", dialog));
		const confirm = byRole("region", "Take it back", dialog);
		expect(confirm.textContent).toContain(
			"If it can't reach the hub, it can stay connected until its cloud access ends. The bot token stays on edge-berlin-01: to cut the bot off for certain, replace the token with the Discord Developer Portal.",
		);
	});

	test("kept off by its provider: assigned, but not connected, and why", async () => {
		const { cell, where, served } = await openWhere(SHOP, TELEGRAM, {
			devices: withShop({
				bots: [{ ...HELPER, state: "token_refused" }, SUPPORT_BOT],
			}),
			schedules: [claimed(TELEGRAM)],
			grants: [approval()],
		});
		const sentence =
			"Assigned to edge-berlin-01 › shop-assistant, but it isn't connected: its token was refused.";
		expect(
			cell.querySelector("[data-runs-on-where='device_idle']")?.textContent,
		).toBe(sentence);
		expect(where()?.textContent).toContain(sentence);
		expect(runLines(served())).toEqual([
			"Not connected: its token was refused",
		]);
	});

	test("its cloud access ended: nothing takes it back by itself, so the action is here", async () => {
		const { where, dialog } = await openWhere(SHOP, TELEGRAM, {
			devices: withShop(),
			schedules: [claimed(TELEGRAM)],
			grants: [approval("revoked")],
		});
		expect(where()?.textContent).toMatch(
			/^Where it runsAssigned to edge-berlin-01 › shop-assistant, but its cloud access ended .+\. Nothing takes it back by itself: take it back here\./,
		);
		expect(byRole("button", "Take it back", dialog)).toBeTruthy();
	});
});

describe("A one-time schedule on the Events list", () => {
	test("ahead of its time on a device: it runs once, and the way back says from when the hub has it", async () => {
		const givenBack: string[] = [];
		const { cell, where, served, dialog } = await openWhere(SHOP, PRICES, {
			devices: withShop(),
			schedules: [claimed(PRICES)],
			grants: [approval()],
			givenBack,
		});
		expect(cell.querySelector("[data-runs-on-next]")?.textContent).toMatch(
			/^Runs once Oct 15 at 09:00 GMT\+2 · in 2 wk\./,
		);
		expect(where()?.getAttribute("data-where-kind")).toBe("once");
		expect(where()?.textContent).toContain(
			"Runs on edge-berlin-01, not on the hub.",
		);
		expect(runLines(served())[0]).toMatch(/^Runs once Oct 15 at 09:00 GMT\+2/);
		await click(byRole("button", "Run it on the hub again", dialog));
		const confirm = byRole("region", "Run it on the hub again", dialog);
		// The service runs: the hub waits for its lease, about 65 minutes.
		expect(confirm.textContent).toContain(
			"It goes back to the hub from 13:05. If its time is before that, nobody runs it.",
		);
		await click(byRole("button", "Run it on the hub again", confirm));
		await settle();
		expect(givenBack).toEqual([PRICES]);
		expect(where()?.textContent).toContain(
			"It goes back to the hub from 12:05. If its time is before that, nobody runs it.",
		);
	});

	test("after it ran it never reads “Runs on”: Ran on the device, also while the service is stopped", async () => {
		const ran: ServiceSchedule = {
			...PENDING,
			once_state: "ran",
			next_at: null,
			last_at: SHOP_ONCE_AT + 4,
			last_outcome: "succeeded",
		};
		for (const patch of [
			{},
			{ desired: "stopped", observed: "stopped", conv: "stopped_by_user" },
		] as Partial<ServiceView>[]) {
			const { cell, where, served, dialog, unmount } = await openWhere(
				SHOP,
				PRICES,
				{
					devices: withShop({ schedules: [ran], ...patch }),
					schedules: [claimed(PRICES)],
					grants: [approval()],
					now: SHOP_ONCE_AT + 3_600,
				},
			);
			const sentence = /^Ran on edge-berlin-01 .+\. Nothing runs it again\.$/;
			expect(
				cell.querySelector("[data-runs-on-where='device']")?.textContent,
			).toMatch(sentence);
			expect(where()?.querySelector("p + p")?.textContent).toMatch(sentence);
			expect(dialog.textContent).not.toContain("Runs on edge-berlin-01");
			expect(runLines(served())).toEqual([
				expect.stringMatching(/^Ran .+ · succeeded$/),
				"Nothing more to run.",
			]);
			await click(byRole("button", "Run it on the hub again", dialog));
			expect(
				byRole("region", "Run it on the hub again", dialog).textContent,
			).toContain(
				"It goes back to the hub. Its time has passed, so nothing runs it again.",
			);
			await unmount();
		}
	});

	test("missed, or its time had passed: did not run, and why", async () => {
		const missed = await openWhere(SHOP, PRICES, {
			devices: withShop({
				schedules: [{ ...PENDING, once_state: "missed", next_at: null }],
			}),
			schedules: [claimed(PRICES)],
			grants: [approval()],
			now: SHOP_ONCE_AT + 3_600,
		});
		expect(missed.where()?.textContent).toMatch(
			/Did not run: edge-berlin-01 wasn't running at its time \(.+\)\./,
		);
		await missed.unmount();
		const passed = await openWhere(SHOP, PRICES, {
			devices: withShop({
				schedules: [{ ...PENDING, once_state: "passed", next_at: null }],
			}),
			schedules: [claimed(PRICES)],
			grants: [approval()],
			now: SHOP_ONCE_AT + 3_600,
		});
		expect(passed.where()?.textContent).toMatch(
			/Did not run: its time \(.+\) had passed when shop-assistant could first run it\./,
		);
	});

	test("a new time set in Events waits for an update of the service", async () => {
		const { served } = await openWhere(SHOP, PRICES, {
			devices: withShop({
				schedules: [
					{
						...PENDING,
						once_at: SHOP_ONCE_AT - 86_400,
						next_at: SHOP_ONCE_AT - 86_400,
					},
				],
			}),
			schedules: [claimed(PRICES)],
			grants: [approval()],
		});
		expect(
			served()?.querySelector("[data-schedule-changed]")?.textContent,
		).toBe(
			"A new time was set in Events. Update shop-assistant to apply it; until then nothing runs it.",
		);
	});
});

describe("Run now… for a form a device runs", () => {
	const stopped: GateFailure = {
		ok: false,
		gate: "G8",
		kind: "busy",
		hide: false,
		copy: { code: "service_must_be_running", params: { service: SERVICE } },
	};

	/** The form as a device reports it when none of its fields is a file. */
	const noFile = () =>
		withShop({
			actions: [{ event_id: FORM, kind: "form", fields: 2, file_fields: 0 }],
		});

	test("beside Open on the device that serves it, and only for a person-started event", async () => {
		const opened: string[] = [];
		const { served, unmount } = await openWhere(SHOP, FORM, {
			devices: noFile(),
			opened,
		});
		const row = served();
		const runNow = byRole("button", "Run now…", row ?? undefined);
		expect(byRole("link", "Open", row ?? undefined)).toBeTruthy();
		expect(runNow.getAttribute("aria-disabled")).toBeNull();
		await click(runNow);
		// The sheet opens over the page; nothing navigates.
		expect(useOverlayStore.getState().overlay).toEqual({
			kind: "run_now",
			deviceId: EDGE,
			serviceId: SERVICE,
			eventId: FORM,
		});
		expect(opened).toEqual([]);
		await unmount();
		for (const eventId of [ORDERS, TELEGRAM, PRICES]) {
			const other = await openWhere(SHOP, eventId, { devices: withShop() });
			expect(
				[eventId, queryByRole("button", "Run now…", other.dialog)].join(),
			).toBe(`${eventId},`);
			await other.unmount();
		}
	});

	test("off with the gate's reason, and it opens nothing", async () => {
		const opened: string[] = [];
		const { served } = await openWhere(SHOP, FORM, {
			devices: noFile(),
			runGate: () => stopped,
			opened,
		});
		const runNow = byRole("button", "Run now…", served() ?? undefined);
		expect(runNow.getAttribute("aria-disabled")).toBe("true");
		const reason = document.getElementById(
			runNow.getAttribute("aria-describedby") ?? "",
		);
		expect(reason?.textContent).toBe("shop-assistant needs to be running.");
		await click(runNow);
		expect(useOverlayStore.getState().overlay).toEqual({ kind: "none" });
		expect(opened).toEqual([]);
	});

	test("a form that takes a file is off: only its service page can send one", async () => {
		const { served } = await openWhere(SHOP, FORM, { devices: withShop() });
		const runNow = byRole("button", "Run now…", served() ?? undefined);
		expect(runNow.getAttribute("aria-disabled")).toBe("true");
		const reason = document.getElementById(
			runNow.getAttribute("aria-describedby") ?? "",
		);
		expect(reason?.textContent).toBe(
			"This form takes a file. Open it on the service page.",
		);
		await click(runNow);
		expect(useOverlayStore.getState().overlay).toEqual({ kind: "none" });
	});
});
