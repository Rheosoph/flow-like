import { describe, expect, test } from "bun:test";
import {
	APPS,
	NOW0,
	PLACEMENTS,
	SERVICES,
	SHOP_ONCE_AT,
	sampleDevices,
	svc,
} from "../../../../lib/device-management/model/__fixtures__/apps";
import { buildAppView } from "../../../../lib/device-management/model/app-plan";
import type {
	ServiceBot,
	ServiceSchedule,
	ServiceView,
} from "../../../../lib/device-management/model/types";
import {
	type RunsOnRow,
	runsOnDeviceNames,
	runsOnRows,
	scheduleOnDevice,
} from "./runs-on-model";

type AppId = keyof typeof APPS;

function viewOf(appId: AppId, options: { labUnlocked?: boolean } = {}) {
	const devices = sampleDevices(options);
	return buildAppView({
		app: APPS[appId],
		devices,
		placements: PLACEMENTS[appId],
		focusDeviceIds: devices.map((device) => device.id),
	});
}

const rowOf = (appId: AppId, eventId: string, labUnlocked = false) => {
	const row = runsOnRows(viewOf(appId, { labUnlocked })).get(eventId);
	if (!row) throw new Error(`no row for ${eventId}`);
	return row;
};

const devicesOf = (list: readonly { deviceId: string }[]) =>
	list.map((entry) => entry.deviceId);

describe("runsOnRows", () => {
	test("splits an event's devices into serving, unknown and elsewhere", () => {
		const row = rowOf("app_invoice_ai", "evt_extract_http");
		expect(devicesOf(row.served)).toEqual(["edge-berlin-01"]);
		expect(row.served[0].service?.serviceId).toBe("invoice-extractor");
		expect(row.served[0].cell.conv).toBe("update_in_progress");
		expect(devicesOf(row.unknown)).toEqual(["lab-gpu-02"]);
		expect(row.unknown[0].unknown.kind).toBe("locked");
		expect(row.lockedOnly).toBe(true);
		expect(devicesOf(row.elsewhere)).toEqual([
			"studio-mac-mini",
			"warehouse-pi",
			"cold-storage-nas",
		]);
	});

	test("counts the devices that serve an older pin than the event's newest", () => {
		const row = rowOf("app_invoice_ai", "evt_extract_http");
		expect(row.older).toBe(1);
		expect(row.pin?.eventVersion).toEqual([1, 5, 0]);
		expect(row.served[0].cell.pin?.eventVersion).toEqual([1, 4, 0]);
		expect(rowOf("app_invoice_ai", "evt_gpu_extract").older).toBe(0);
	});

	test("an unlocked shared device moves from unknown to serving", () => {
		const row = rowOf("app_invoice_ai", "evt_gpu_extract", true);
		expect(devicesOf(row.served)).toEqual(["lab-gpu-02"]);
		expect(row.served[0].service?.serviceId).toBe("invoice-extractor-gpu");
		expect(row.unknown).toEqual([]);
		expect(row.older).toBe(1);
	});

	test("a device that never checked in is never unknown: nothing can run there", () => {
		for (const row of runsOnRows(viewOf("app_invoice_ai")).values()) {
			expect(devicesOf(row.unknown)).not.toContain("cold-storage-nas");
			expect(
				row.elsewhere.find((entry) => entry.deviceId === "cold-storage-nas")
					?.state,
			).toBe("not_served");
		}
	});

	test("a status without an event list is unknown and names the service it does list", () => {
		const row = rowOf("app_warehouse_scan", "evt_scan_station");
		expect(row.served).toEqual([]);
		expect(row.lockedOnly).toBe(false);
		expect(row.unknown.map((entry) => entry.unknown.kind)).toEqual([
			"snapshot",
			"locked",
		]);
		expect(row.unknown[0].serviceId).toBe("scanner-ingest");
		expect(row.unknown[1].serviceId).toBeUndefined();
	});

	test("keeps the device's own refusal for the popover", () => {
		const refused = rowOf("app_crm_sync", "evt_crm_watch").elsewhere.find(
			(entry) => entry.state === "cant_here",
		);
		expect(refused?.deviceId).toBe("edge-berlin-01");
		expect(refused?.reason).toContain("sandboxed");
	});

	test("events that can't run on devices have no row; a schedule has one", () => {
		const rows = runsOnRows(viewOf("app_crm_sync"));
		expect([...rows.keys()].sort()).toEqual([
			"evt_crm_hourly",
			"evt_crm_nightly",
			"evt_crm_watch",
			"evt_crm_webhook",
		]);
	});

	test("a schedule carries when it runs and, for an online app, where", () => {
		const local = rowOf("app_crm_sync", "evt_crm_hourly");
		expect(local.schedule).toEqual({
			expression: "0 0 * * * *",
			timezone: "UTC",
			zoneSet: false,
		});
		expect(local.where).toBeUndefined();
		const online = rowOf("app_invoice_ai", "evt_invoice_reconcile");
		expect(online.schedule?.timezone).toBe("Europe/Berlin");
		expect(online.where).toEqual({ fact: "hub" });
		expect(
			rowOf("app_invoice_ai", "evt_extract_http").schedule,
		).toBeUndefined();
		expect(online.deviceOnly).toBeUndefined();
	});

	test("a device-only schedule carries the marker and a device-only fact, not the hub's", () => {
		const base = APPS.app_invoice_ai;
		const devices = sampleDevices();
		const view = buildAppView({
			app: {
				...base,
				events: base.events.map((event) =>
					event.id === "evt_invoice_reconcile"
						? { ...event, deviceOnly: true }
						: event,
				),
			},
			devices,
			placements: PLACEMENTS.app_invoice_ai,
			focusDeviceIds: devices.map((device) => device.id),
		});
		const row = runsOnRows(view).get("evt_invoice_reconcile");
		expect(row?.deviceOnly).toBe(true);
		expect(row?.where).toEqual({ fact: "device_only" });
	});

	test("an event that follows Latest says so", () => {
		expect(rowOf("app_invoice_ai", "evt_invoice_review").followsLatest).toBe(
			true,
		);
		expect(rowOf("app_invoice_ai", "evt_extract_http").followsLatest).toBe(
			false,
		);
	});

	test("an event that can't run any more keeps its row while a device still serves it", () => {
		const paused = {
			...APPS.app_support_portal,
			events: APPS.app_support_portal.events.map((event) =>
				event.id === "evt_support_http" ? { ...event, active: false } : event,
			),
		};
		const devices = sampleDevices();
		const rows = runsOnRows(
			buildAppView({
				app: paused,
				devices,
				placements: PLACEMENTS.app_support_portal,
				focusDeviceIds: devices.map((device) => device.id),
			}),
		);
		expect(devicesOf(rows.get("evt_support_http")?.served ?? [])).toEqual([
			"edge-berlin-01",
		]);
		expect(rows.has("evt_support_mailbox")).toBe(false);
	});

	test("the cell's counts don't depend on listing every device as a column", () => {
		const devices = sampleDevices();
		const narrow = runsOnRows(
			buildAppView({
				app: APPS.app_invoice_ai,
				devices,
				placements: PLACEMENTS.app_invoice_ai,
			}),
		);
		for (const [eventId, row] of runsOnRows(viewOf("app_invoice_ai"))) {
			expect(devicesOf(narrow.get(eventId)?.served ?? [])).toEqual(
				devicesOf(row.served),
			);
			expect(devicesOf(narrow.get(eventId)?.unknown ?? [])).toEqual(
				devicesOf(row.unknown),
			);
		}
	});
});

describe("scheduleOnDevice: a schedule this place doesn't run", () => {
	const EVENT = "evt_invoice_reconcile";
	const ASSIGNED = {
		deviceId: "edge-berlin-01",
		serviceId: "invoice-extractor",
		since: 1,
		seenAt: 2,
	};
	const ARMED: ServiceSchedule = {
		event_id: EVENT,
		expression: "0 0 2 * * *",
		timezone: "Europe/Berlin",
		hold: null,
	};
	const base = rowOf("app_invoice_ai", EVENT);
	const onDevice = (patch: Partial<RunsOnRow>) =>
		scheduleOnDevice({ ...base, ...patch });
	const serving = (view: Partial<ServiceView>): RunsOnRow["served"] => [
		{
			deviceId: "edge-berlin-01",
			device: "edge-berlin-01",
			cell: {
				state: "served",
				deviceId: "edge-berlin-01",
				serviceIds: ["invoice-extractor"],
			},
			service: {
				view: svc({ ...SERVICES.invoiceExtractor, conv: "converged", ...view }),
			} as RunsOnRow["served"][number]["service"],
		},
	];

	test("an online app: the hub says who has it; only a service known to run it reads as running", () => {
		expect(
			onDevice({
				where: { fact: "device", ...ASSIGNED, schedule: ARMED, live: true },
			}),
		).toBe("runs");
		// Assigned to a service this computer can't read: "not running" can't be said either.
		expect(
			onDevice({
				where: {
					fact: "device_unconfirmed",
					since: 1,
					seenAt: 2,
					stale: false,
					readable: false,
				},
			}),
		).toBe("assigned");
		expect(
			onDevice({ where: { fact: "device_idle", why: "stopped", ...ASSIGNED } }),
		).toBeNull();
		expect(onDevice({ where: { fact: "released", since: 1 } })).toBeNull();
		expect(onDevice({ where: { fact: "hub" } })).toBeNull();
		expect(
			onDevice({ where: { fact: "returning", hubResumesAt: 9 } }),
		).toBeNull();
	});

	test("a local-only app has no hub list: a readable service that reports the schedule armed runs it", () => {
		const { where: _where, ...local } = base;
		const of = (view: Partial<ServiceView>) =>
			scheduleOnDevice({ ...local, served: serving(view) });
		expect(of({ schedules: [ARMED] })).toBe("runs");
		expect(of({ schedules: [{ ...ARMED, hold: "other_service" }] })).toBeNull();
		expect(of({ schedules: [ARMED], desired: "stopped" })).toBeNull();
		expect(of({ schedules: "not_reported" })).toBeNull();
		expect(scheduleOnDevice({ ...local, served: [] })).toBeNull();
	});

	test("an event that is not a schedule never gets the chip", () => {
		expect(
			scheduleOnDevice(rowOf("app_invoice_ai", "evt_extract_http")),
		).toBeNull();
		expect(scheduleOnDevice(undefined)).toBeNull();
	});
});

describe("round two: the new kinds on the Events list (design R2 §6.4)", () => {
	const SHOP = "app_shop_assistant";
	const ASSIGNED = {
		deviceId: "edge-berlin-01",
		serviceId: "shop-assistant",
		since: 1,
		seenAt: 2,
	};
	const BOT: ServiceBot = {
		event_id: "evt_shop_telegram",
		provider: "telegram",
		state: "connected",
		hold: null,
		bot_name: "helper_bot",
	};
	const ONCE: ServiceSchedule = {
		event_id: "evt_shop_prices",
		once_at: SHOP_ONCE_AT,
		once_state: "ran",
		timezone: "Europe/Berlin",
		hold: null,
		last_at: SHOP_ONCE_AT + 4,
		last_outcome: "succeeded",
	};

	test("every new kind is a row with its kind; a one-time schedule carries its instant, not an expression", () => {
		const rows = runsOnRows(viewOf(SHOP));
		expect(
			Object.fromEntries([...rows].map(([id, row]) => [id, row.kind])),
		).toEqual({
			evt_shop_orders: "served",
			evt_shop_return: "on_demand",
			evt_shop_telegram: "bot",
			evt_shop_discord: "bot",
			evt_shop_prices: "scheduled",
		});
		const prices = rowOf(SHOP, "evt_shop_prices");
		expect(prices.once?.at).toBe(SHOP_ONCE_AT);
		expect(prices.schedule).toBeUndefined();
		expect(prices.where).toEqual({ fact: "hub" });
		expect(rowOf(SHOP, "evt_shop_telegram").where).toEqual({ fact: "hub" });
		expect(rowOf(SHOP, "evt_shop_return").where).toBeUndefined();
	});

	test("the chip: a bot a device keeps connected runs there; one kept off does not", () => {
		const bot = rowOf(SHOP, "evt_shop_telegram");
		expect(
			scheduleOnDevice({
				...bot,
				where: { fact: "device", ...ASSIGNED, bot: BOT, live: true },
			}),
		).toBe("runs");
		expect(
			scheduleOnDevice({
				...bot,
				where: {
					fact: "device_idle",
					why: "not_connected",
					botState: "token_refused",
					...ASSIGNED,
				},
			}),
		).toBeNull();
		expect(scheduleOnDevice(rowOf(SHOP, "evt_shop_return"))).toBeNull();
	});

	test("the chip: a one-time schedule that ran reads ran, never runs; one that was missed reads neither", () => {
		const prices = rowOf(SHOP, "evt_shop_prices");
		const finished = (state: "ran" | "missed") =>
			scheduleOnDevice({
				...prices,
				where: {
					fact: "device",
					...ASSIGNED,
					schedule: { ...ONCE, once_state: state },
					live: true,
					finished: { state, at: SHOP_ONCE_AT },
				},
			});
		expect(finished("ran")).toBe("ran");
		expect(finished("missed")).toBeNull();
		const { where: _where, ...local } = prices;
		const served = (schedules: ServiceSchedule[]): RunsOnRow["served"] => [
			{
				deviceId: "edge-berlin-01",
				device: "edge-berlin-01",
				cell: {
					state: "served",
					deviceId: "edge-berlin-01",
					serviceIds: ["shop-assistant"],
				},
				service: {
					view: svc({
						deviceId: "edge-berlin-01",
						serviceId: "shop-assistant",
						projectId: SHOP,
						desired: "stopped",
						conv: "stopped_by_user",
						schedules,
					}),
				} as RunsOnRow["served"][number]["service"],
			},
		];
		// Without a hub list: the stopped service still reports the finished entry.
		expect(scheduleOnDevice({ ...local, served: served([ONCE]) })).toBe("ran");
		expect(
			scheduleOnDevice({
				...local,
				served: served([{ ...ONCE, once_state: "pending" }]),
			}),
		).toBeNull();
	});

	test("a device whose agent lacks the flag keeps the flag for the popover; a bot elsewhere says so", () => {
		const devices = sampleDevices().map((device) =>
			device.id === "studio-mac-mini"
				? { ...device, features: { scheduled_events: 1 as const } }
				: device,
		);
		const rows = runsOnRows(
			buildAppView({
				app: APPS[SHOP],
				devices,
				placements: {
					...PLACEMENTS[SHOP],
					schedules: [
						{
							event_id: "evt_shop_telegram",
							state: "device",
							since: NOW0 - 60,
							seen_at: NOW0 - 30,
							device_id: "warehouse-pi",
							placement_id: "shop-bots",
						},
					],
				},
				focusDeviceIds: devices.map((device) => device.id),
				now: NOW0,
			}),
		);
		const studio = (eventId: string) =>
			rows
				.get(eventId)
				?.elsewhere.find((entry) => entry.deviceId === "studio-mac-mini");
		expect(studio("evt_shop_return")).toMatchObject({
			state: "cant_here",
			why: "agent",
			feature: "on_demand_events",
		});
		expect(studio("evt_shop_orders")?.feature).toBe("api_events");
		const edge = rows
			.get("evt_shop_telegram")
			?.elsewhere.find((entry) => entry.deviceId === "edge-berlin-01");
		expect(edge).toMatchObject({
			state: "cant_here",
			why: "runs_elsewhere",
			bot: true,
		});
	});
});

describe("runsOnDeviceNames", () => {
	test("names every device the app view mentions", () => {
		const names = runsOnDeviceNames(viewOf("app_invoice_ai"));
		expect([...names.keys()].sort()).toEqual([
			"cold-storage-nas",
			"edge-berlin-01",
			"lab-gpu-02",
			"studio-mac-mini",
			"warehouse-pi",
		]);
	});
});
