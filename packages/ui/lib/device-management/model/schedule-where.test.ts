import { describe, expect, test } from "bun:test";
import { NOW0, PLACEMENTS, SERVICES, svc } from "./__fixtures__/apps";
import {
	SCHEDULE_CONFIRM_STALE_S,
	type ScheduleWhereDevice,
	holdsSchedule,
	isAssignedTo,
	runsSchedule,
	scheduleRun,
	scheduleWhere,
	scheduleWheres,
	schedulesClaimedBy,
	whereOf,
} from "./schedule-where";
import type {
	AppDevicePlacements,
	AppScheduleRow,
	Freshness,
	ServiceBot,
	ServiceSchedule,
	ServiceView,
} from "./types";

/* Where a schedule runs: every row of the table in run-more-design §2.5. */

const EVENT = "evt_invoice_reconcile";
const DEVICE = "edge-berlin-01";
const SERVICE = "invoice-extractor";
const CLAIMED: Extract<AppScheduleRow, { state: "device" }> = {
	event_id: EVENT,
	state: "device",
	since: NOW0 - 86_400,
	seen_at: NOW0 - 600,
	grant_id: "grant-invoice-extractor",
	device_id: DEVICE,
	placement_id: SERVICE,
};
const ARMED: ServiceSchedule = {
	event_id: EVENT,
	expression: "0 0 2 * * *",
	timezone: "Europe/Berlin",
	hold: null,
	last_outcome: "succeeded",
};
const SNAPSHOT: Freshness = { src: "snap", age: "current", at: NOW0 - 40 };

function runs(seed: Partial<ServiceView> = {}): ServiceView {
	return svc({
		...SERVICES.invoiceExtractor,
		observed: "running",
		conv: "converged",
		schedules: [{ ...ARMED, next_at: NOW0 + 3_600, runs: 12 }],
		...seed,
	});
}

const placements = PLACEMENTS.app_invoice_ai;
const where = (
	services: ScheduleWhereDevice["services"],
	row: AppScheduleRow = CLAIMED,
	hub: Pick<AppDevicePlacements, "placements"> = placements,
) =>
	scheduleWhere(row, {
		placements: hub,
		devices: [{ id: DEVICE, services }],
		now: NOW0,
	});

describe("a schedule a service claimed", () => {
	test("runs on the device only while a running service is known to run it", () => {
		expect(where([runs()])).toEqual({
			fact: "device",
			deviceId: DEVICE,
			serviceId: SERVICE,
			since: NOW0 - 86_400,
			seenAt: NOW0 - 600,
			schedule: { ...ARMED, next_at: NOW0 + 3_600, runs: 12 },
			live: true,
		});
		// A snapshot that is not stale says it too, without times and counters.
		expect(
			where([runs({ freshness: SNAPSHOT, schedules: [ARMED] })]),
		).toMatchObject({ fact: "device", live: false, schedule: ARMED });
		expect(
			where([
				runs({
					freshness: { src: "snap", age: "delayed", at: NOW0 - 300 },
					schedules: [ARMED],
				}),
			]).fact,
		).toBe("device");
	});

	test("is assigned but idle when the service is stopped, failing, holds it, is gone or lost its cloud access", () => {
		const idle = (services: ServiceView[]) => {
			const fact = where(services);
			return fact.fact === "device_idle" ? fact : null;
		};
		expect(
			idle([runs({ desired: "stopped", conv: "stopped_by_user" })]),
		).toMatchObject({ why: "stopped", deviceId: DEVICE, serviceId: SERVICE });
		expect(idle([runs({ desired: "stopped", conv: "converging" })])?.why).toBe(
			"stopped",
		);
		for (const conv of ["crash_looping", "failed_stopped"] as const)
			expect(idle([runs({ conv, schedules: "not_reported" })])?.why).toBe(
				"failed",
			);
		expect(
			idle([runs({ schedules: [{ ...ARMED, hold: "not_released" }] })]),
		).toMatchObject({ why: "held", hold: "not_released" });
		expect(idle([SERVICES.supportBot])?.why).toBe("removed");
		expect(idle([])?.why).toBe("removed");
	});

	test("its cloud access ended: the approval's own facts say so before the hub notices", () => {
		const [approval] = placements.placements;
		const ended = (grant: Partial<typeof approval.grant>) =>
			where([runs()], CLAIMED, {
				placements: [{ ...approval, grant: { ...approval.grant, ...grant } }],
			});
		expect(ended({ effective_expires_at: NOW0 - 60 })).toEqual({
			fact: "device_idle",
			why: "access_ended",
			endedAt: NOW0 - 60,
			deviceId: DEVICE,
			serviceId: SERVICE,
			since: NOW0 - 86_400,
			seenAt: NOW0 - 600,
		});
		expect(ended({ status: "revoked" })).toMatchObject({
			why: "access_ended",
		});
		expect(ended({ effective_expires_at: NOW0 + 60 }).fact).toBe("device");
		// An approval the viewer can't see says nothing: the device's state decides.
		expect(where([runs()], CLAIMED, { placements: [] }).fact).toBe("device");
	});

	test("is unconfirmed while the device's state is not known, and stale after two hours without a confirmation", () => {
		const unconfirmed = {
			fact: "device_unconfirmed",
			deviceId: DEVICE,
			serviceId: SERVICE,
			since: NOW0 - 86_400,
			seenAt: NOW0 - 600,
			stale: false,
		};
		// A device that can't be read: unlocking it may tell.
		for (const services of [{ state: "locked" }, { state: "notloaded" }])
			expect(where(services)).toEqual({
				...unconfirmed,
				readable: false,
			} as never);
		// Readable, and it does not say: no fresh status, a row without the schedule fact, an agent too old to say.
		const unknown: ScheduleWhereDevice["services"][] = [
			[
				runs({
					freshness: { src: "snap", age: "lastknown", at: NOW0 - 9_000 },
				}),
			],
			[runs({ freshness: { src: "saved", age: "snapshot" } })],
			[runs({ schedules: "not_reported" })],
			[runs({ schedules: "needs_agent" })],
			[runs({ schedules: "not_loaded" })],
			[runs({ schedules: [{ ...ARMED, event_id: "evt_other" }] })],
			[runs({ schedules: undefined })],
		];
		for (const services of unknown)
			expect(where(services)).toEqual({
				...unconfirmed,
				readable: true,
			} as never);
		// A stale list never proves that the service was removed.
		expect(
			where([
				{
					...SERVICES.supportBot,
					freshness: { src: "snap", age: "lastknown", at: NOW0 - 9_000 },
				},
			]).fact,
		).toBe("device_unconfirmed");
		const quiet = (age: number) =>
			where({ state: "locked" }, { ...CLAIMED, seen_at: NOW0 - age });
		expect(quiet(SCHEDULE_CONFIRM_STALE_S)).toMatchObject({ stale: false });
		expect(quiet(SCHEDULE_CONFIRM_STALE_S + 1)).toMatchObject({
			stale: true,
			seenAt: NOW0 - SCHEDULE_CONFIRM_STALE_S - 1,
		});
	});

	test("on a device the viewer can't see carries no ids and is unconfirmed", () => {
		const hidden: AppScheduleRow = {
			event_id: EVENT,
			state: "device",
			since: NOW0 - 86_400,
			seen_at: NOW0 - 3 * 3_600,
		};
		expect(where([runs()], hidden)).toEqual({
			fact: "device_unconfirmed",
			since: NOW0 - 86_400,
			seenAt: NOW0 - 3 * 3_600,
			stale: true,
			readable: false,
		});
		expect(
			scheduleWhere(CLAIMED, { placements, devices: [], now: NOW0 }),
		).toMatchObject({ fact: "device_unconfirmed", readable: false });
	});
});

describe("a schedule that is released, returning or on the hub", () => {
	test("released: the hub runs it until that service starts it", () => {
		expect(
			where([runs()], {
				event_id: EVENT,
				state: "released",
				since: NOW0 - 30,
				device_id: DEVICE,
				placement_id: SERVICE,
			}),
		).toEqual({
			fact: "released",
			since: NOW0 - 30,
			deviceId: DEVICE,
			serviceId: SERVICE,
		});
		expect(
			where([runs()], {
				event_id: EVENT,
				state: "released",
				since: NOW0 - 30,
				hub_resumes_at: NOW0 + 240,
			}),
		).toEqual({ fact: "released", since: NOW0 - 30, hubResumesAt: NOW0 + 240 });
	});

	test("returning: the hub runs it again at the resume time", () => {
		expect(
			where([runs()], {
				event_id: EVENT,
				state: "returning",
				hub_resumes_at: NOW0 + 300,
			}),
		).toEqual({ fact: "returning", hubResumesAt: NOW0 + 300 });
	});

	test("a schedule the hub runs is not listed; an older hub says nothing at all", () => {
		const input = { devices: [{ id: DEVICE, services: [runs()] }], now: NOW0 };
		const wheres = scheduleWheres(
			{ ...placements, schedules: [CLAIMED] },
			input,
		);
		expect(Object.keys(wheres ?? {})).toEqual([EVENT]);
		expect(whereOf(wheres, EVENT)?.fact).toBe("device");
		expect(whereOf(wheres, "evt_other")).toEqual({ fact: "hub" });
		expect(whereOf(wheres, "toString")).toEqual({ fact: "hub" });
		const { schedules: _schedules, ...older } = placements;
		expect(scheduleWheres(older, input)).toBeNull();
		expect(whereOf(null, EVENT)).toBeUndefined();
		expect(whereOf(undefined, EVENT)).toBeUndefined();
	});

	test("a device-only event the hub does not list, released or hands back is device-only, never the hub; a service that holds it still shows", () => {
		const input = { devices: [{ id: DEVICE, services: [runs()] }], now: NOW0 };
		const wheres = scheduleWheres(
			{
				...placements,
				schedules: [
					CLAIMED,
					{
						event_id: "evt_back",
						state: "returning",
						hub_resumes_at: NOW0 + 9,
					},
					{ event_id: "evt_rel", state: "released", since: NOW0 },
				],
			},
			input,
		);
		const deviceOnly = { fact: "device_only" } as const;
		expect(whereOf(wheres, "evt_unlisted", true)).toEqual(deviceOnly);
		expect(whereOf(wheres, "evt_back", true)).toEqual(deviceOnly);
		expect(whereOf(wheres, "evt_rel", true)).toEqual(deviceOnly);
		expect(whereOf(wheres, EVENT, true)?.fact).toBe("device");
		expect(whereOf(wheres, "evt_unlisted")).toEqual({ fact: "hub" });
		expect(whereOf(wheres, "evt_back")?.fact).toBe("returning");
		expect(whereOf(null, "evt_unlisted", true)).toBeUndefined();
	});
});

describe("the schedules a service took off the hub", () => {
	test("only what it claimed: a release it never picked up still runs on the hub", () => {
		const rows: AppScheduleRow[] = [
			{ ...CLAIMED, event_id: "evt_weekly" },
			CLAIMED,
			{ ...CLAIMED, event_id: "evt_other_service", placement_id: "reports" },
			{ ...CLAIMED, event_id: "evt_other_device", device_id: "lab-03" },
			{
				event_id: "evt_released",
				state: "released",
				since: NOW0,
				device_id: DEVICE,
				placement_id: SERVICE,
			},
			{ event_id: "evt_back", state: "returning", hub_resumes_at: NOW0 + 300 },
		];
		expect(schedulesClaimedBy(rows, DEVICE, SERVICE)).toEqual([
			EVENT,
			"evt_weekly",
		]);
		expect(schedulesClaimedBy(undefined, DEVICE, SERVICE)).toEqual([]);
	});
});

describe("what a schedule of a service does now", () => {
	const run = (seed: Partial<ServiceView>) =>
		scheduleRun(runs(seed), EVENT, NOW0);

	test("armed: the device's own next run while live, a computed one on a status that is not stale, none on a stale one", () => {
		expect(run({})).toEqual({
			state: "armed",
			entry: { ...ARMED, next_at: NOW0 + 3_600, runs: 12 },
			next: { at: NOW0 + 3_600, computed: false },
		});
		// 02:00 in Berlin after 2026-09-30 12:00 UTC is 2026-10-01 00:00 UTC.
		expect(run({ freshness: SNAPSHOT, schedules: [ARMED] })).toMatchObject({
			state: "armed",
			next: { at: Date.UTC(2026, 9, 1) / 1_000, computed: true },
		});
		expect(
			run({
				freshness: { src: "snap", age: "lastknown", at: NOW0 - 9_000 },
				schedules: [ARMED],
			}),
		).toMatchObject({ state: "armed", next: null, stale: true });
		// The device said there is no next run: none is shown, never a guess.
		const none = run({ schedules: [{ ...ARMED, next_at: null }] });
		expect(none).toMatchObject({ state: "armed", next: null });
		expect("stale" in none).toBe(false);
	});

	test("held: no next run, and the hold says why", () => {
		expect(
			run({ schedules: [{ ...ARMED, hold: "runs_elsewhere", next_at: 5 }] }),
		).toEqual({
			state: "held",
			hold: "runs_elsewhere",
			entry: { ...ARMED, hold: "runs_elsewhere", next_at: 5 },
		});
	});

	test("without the fact: stopped, not reported yet, an agent too old to say, or unknown", () => {
		expect(run({ schedules: "not_reported", desired: "stopped" }).state).toBe(
			"stopped",
		);
		expect(
			run({ schedules: "not_reported", conv: "crash_looping" }).state,
		).toBe("stopped");
		expect(
			run({
				schedules: "not_reported",
				conv: "converging",
				observed: "starting",
			}).state,
		).toBe("not_reported");
		expect(run({ schedules: [] }).state).toBe("not_reported");
		// A row that still carries the numbers of a process that is gone: no next run, no counters.
		expect(run({ desired: "stopped" })).toEqual({ state: "stopped" });
		expect(run({ conv: "failed_stopped" })).toEqual({ state: "stopped" });
		expect(run({ schedules: "needs_agent" }).state).toBe("needs_agent");
		expect(run({ schedules: "not_loaded" }).state).toBe("unknown");
		expect(run({ schedules: undefined }).state).toBe("unknown");
	});
});

describe("a one-time schedule (design R2 §3, §6.2)", () => {
	const ONCE: ServiceSchedule = {
		event_id: EVENT,
		once_at: NOW0 + 3_600,
		once_state: "pending",
		timezone: "Europe/Berlin",
		hold: null,
		last_outcome: null,
	};
	const once = (
		entry: Partial<ServiceSchedule>,
		seed: Partial<ServiceView> = {},
	) => runs({ schedules: [{ ...ONCE, ...entry }], ...seed });

	test("pending runs at its instant; it is armed like any schedule", () => {
		expect(where([once({ next_at: NOW0 + 3_600 })])).toMatchObject({
			fact: "device",
			live: true,
		});
		expect(where([once({ next_at: NOW0 + 3_600 })])).not.toHaveProperty(
			"finished",
		);
		expect(scheduleRun(once({}), EVENT, NOW0)).toMatchObject({
			state: "armed",
			next: { at: NOW0 + 3_600, computed: true },
		});
		expect(
			scheduleRun(once({ next_at: NOW0 + 3_600 }), EVENT, NOW0),
		).toMatchObject({ next: { at: NOW0 + 3_600, computed: false } });
	});

	test("finished is a fact of its own, also while its service is stopped: never “device” without it", () => {
		const ran = {
			once_state: "ran",
			last_at: NOW0 - 50,
			last_outcome: "succeeded",
		} as const;
		for (const seed of [
			{},
			{ desired: "stopped", conv: "stopped_by_user" },
		] as const) {
			const fact = where([once(ran, seed)]);
			expect(fact).toMatchObject({
				fact: "device",
				finished: { state: "ran", at: NOW0 - 50, outcome: "succeeded" },
			});
			expect(scheduleRun(once(ran, seed), EVENT, NOW0)).toMatchObject({
				state: "finished",
				finished: { state: "ran" },
			});
		}
		for (const state of ["missed", "passed"] as const)
			expect(where([once({ once_state: state })])).toMatchObject({
				fact: "device",
				finished: { state, at: NOW0 + 3_600 },
			});
		// A snapshot row without a run time: its time stands in for when it ran.
		expect(where([once({ once_state: "ran" })])).toMatchObject({
			finished: { state: "ran", at: NOW0 + 3_600 },
		});
		expect(runsSchedule(once(ran), EVENT)).toBe(false);
		expect(runsSchedule(once({}), EVENT)).toBe(true);
	});

	test("a started run is still armed; a held one is idle", () => {
		expect(
			scheduleRun(once({ once_state: "started", running: true }), EVENT, NOW0)
				.state,
		).toBe("armed");
		expect(where([once({ hold: "runs_elsewhere" })])).toMatchObject({
			fact: "device_idle",
			why: "held",
			hold: "runs_elsewhere",
		});
	});
});

describe("a bot a service claimed (design R2 §5.7)", () => {
	const BOT = "evt_helper";
	const row = { ...CLAIMED, event_id: BOT };
	const bot = (entry: Partial<ServiceBot>, seed: Partial<ServiceView> = {}) =>
		runs({
			bots: [
				{
					event_id: BOT,
					provider: "telegram",
					state: "connected",
					hold: null,
					...entry,
				},
			],
			...seed,
		});

	test("connected, connecting or a snapshot's ok: the device runs it", () => {
		for (const state of [
			"connected",
			"connecting",
			"reconnecting",
			"ok",
		] as const)
			expect(where([bot({ state })], row)).toMatchObject({
				fact: "device",
				bot: { state },
			});
		expect(runsSchedule(bot({}), BOT)).toBe(true);
	});

	test("held, refused or used elsewhere: assigned, and nothing answers it", () => {
		expect(
			where([bot({ state: "waiting", hold: "not_released" })], row),
		).toMatchObject({ fact: "device_idle", why: "held", hold: "not_released" });
		for (const botState of [
			"token_refused",
			"intents_refused",
			"conflict",
			"webhook_set",
		] as const) {
			expect(where([bot({ state: botState })], row)).toMatchObject({
				fact: "device_idle",
				why: "not_connected",
				botState,
			});
			expect(runsSchedule(bot({ state: botState }), BOT)).toBe(false);
		}
		expect(
			where([bot({}, { desired: "stopped", conv: "stopped_by_user" })], row),
		).toMatchObject({ fact: "device_idle", why: "stopped" });
		// Waiting without a hold: the service has not decided yet.
		expect(where([bot({ state: "waiting" })], row)).toMatchObject({
			fact: "device_unconfirmed",
			readable: true,
		});
	});
});

test("only a service that holds a schedule blocks another place; the hub and a release do not", () => {
	const held = [
		where([runs()]),
		where([runs({ desired: "stopped" })]),
		where({ state: "locked" }),
	];
	expect(held.map((fact) => fact.fact)).toEqual([
		"device",
		"device_idle",
		"device_unconfirmed",
	]);
	for (const fact of held) {
		expect(holdsSchedule(fact)).toBe(true);
		expect(isAssignedTo(fact, DEVICE, SERVICE)).toBe(true);
		expect(isAssignedTo(fact, DEVICE, "another")).toBe(false);
		expect(isAssignedTo(fact, "lab-gpu-02", SERVICE)).toBe(false);
	}
	for (const fact of [
		{ fact: "hub" },
		{ fact: "released", since: 1, deviceId: DEVICE, serviceId: SERVICE },
		{ fact: "returning", hubResumesAt: 9 },
		undefined,
	] as const)
		expect(holdsSchedule(fact)).toBe(false);
	expect(
		isAssignedTo(
			{ fact: "released", since: 1, deviceId: DEVICE, serviceId: SERVICE },
			DEVICE,
			SERVICE,
		),
	).toBe(true);
	expect(isAssignedTo({ fact: "hub" }, DEVICE, SERVICE)).toBe(false);
	expect(isAssignedTo(undefined, DEVICE, SERVICE)).toBe(false);
});
