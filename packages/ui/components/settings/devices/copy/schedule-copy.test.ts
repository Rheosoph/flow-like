import { describe, expect, test } from "bun:test";
import { getI18n } from "@flow-like/locales";
import { formatRelativeTime } from "../../../../lib/date";
import { NOW0 } from "../../../../lib/device-management/model/__fixtures__/apps";
import type {
	ScheduleRun,
	ScheduleWhere,
} from "../../../../lib/device-management/model/schedule-where";
import {
	SCHEDULE_HOLDS,
	SCHEDULE_OUTCOMES,
	SCHEDULE_SKIP_REASONS,
	type ServiceSchedule,
} from "../../../../lib/device-management/model/types";
import type { DevicesT } from "../primitives/area-context";
import {
	type WhereNames,
	canRunOnHubAgain,
	deviceOnlyText,
	scheduleHead,
	scheduleHoldText,
	scheduleNameList,
	scheduleOutcome,
	scheduleRunLines,
	scheduleRunNames,
	scheduleSkip,
	scheduleTime,
	scheduleWhereText,
	scheduleWords,
	scheduleWordsInline,
	scheduleZone,
} from "./schedule-copy";

const t = getI18n().getFixedT("en", "devices") as DevicesT;

const NAMES: WhereNames = {
	device: (deviceId) => deviceId ?? "a device",
	at: (atS) => `@${atS}`,
	ago: (atS) => `${atS} ago`,
};

const ASSIGNED = {
	deviceId: "edge-berlin-01",
	serviceId: "invoice-reconcile",
	since: 1,
	seenAt: 2,
};

const ENTRY: ServiceSchedule = {
	event_id: "evt_invoice_reconcile",
	expression: "0 0 2 * * *",
	timezone: "Europe/Berlin",
	hold: null,
};

/** The sample's clock: 2026-09-30 12:00 UTC, read in English. */
const TIME = {
	now: NOW0 * 1000,
	locale: "en",
	ago: (atS: number, style: Intl.RelativeTimeFormatStyle = "narrow") =>
		formatRelativeTime(atS * 1000, style, "", {
			now: NOW0 * 1000,
			locale: "en",
		}),
};

const MACHINE_WORDS = /[a-z]_[a-z]/;

const say = (where: ScheduleWhere) => scheduleWhereText(t, where, NAMES);

describe("one-time schedules and bots, as one sentence", () => {
	const ONCE: ServiceSchedule = {
		event_id: "evt_once",
		once_at: 1792047600,
		timezone: "Europe/Berlin",
		hold: null,
	};
	const finished = (
		state: "ran" | "missed" | "passed",
		extra: object = {},
	): ScheduleWhere => ({
		fact: "device",
		schedule: { ...ONCE, once_state: state },
		live: false,
		finished: { state, at: 1792047605, ...extra },
		...ASSIGNED,
	});

	test("a finished one-time schedule says what happened, never “Runs on”", () => {
		expect(scheduleWhereText(t, finished("ran"), NAMES, "once")).toBe(
			"Ran on edge-berlin-01 @1792047605. Nothing runs it again.",
		);
		expect(scheduleWhereText(t, finished("missed"), NAMES, "once")).toBe(
			"Did not run: edge-berlin-01 wasn't running at its time (@1792047605).",
		);
		expect(scheduleWhereText(t, finished("passed"), NAMES, "once")).toBe(
			"Did not run: its time (@1792047605) had passed when invoice-reconcile could first run it.",
		);
		// A pending one reads as round one's schedule does.
		expect(
			scheduleWhereText(
				t,
				{ fact: "device", schedule: ONCE, live: true, ...ASSIGNED },
				NAMES,
				"once",
			),
		).toBe("Runs on edge-berlin-01, not on the hub.");
		// Only some hubs fire a one-time schedule, so it never reads "Runs on the hub".
		expect(scheduleWhereText(t, { fact: "hub" }, NAMES, "once")).toBe(
			"Not on a device: the hub keeps it.",
		);
	});

	test("a bot or one-time schedule whose cloud access ended has no next time to go back at", () => {
		const ended: ScheduleWhere = {
			fact: "device_idle",
			why: "access_ended",
			endedAt: 7,
			...ASSIGNED,
		};
		const sentence =
			"Assigned to edge-berlin-01 › invoice-reconcile, but its cloud access ended @7. Nothing takes it back by itself: take it back here.";
		expect(scheduleWhereText(t, ended, NAMES, "once")).toBe(sentence);
		expect(scheduleWhereText(t, ended, NAMES, "bot")).toBe(sentence);
		expect(scheduleWhereText(t, ended, NAMES)).toContain(
			"the hub takes it back at its next scheduled time",
		);
	});

	test("a device-only event reads the same for a schedule, a one-time schedule and a bot, and never mentions the hub running it", () => {
		const deviceOnly: ScheduleWhere = { fact: "device_only" };
		const sentence =
			"Runs only on the devices you deploy it to. Not deployed yet.";
		for (const kind of ["schedule", "once", "bot"] as const)
			expect(scheduleWhereText(t, deviceOnly, NAMES, kind)).toBe(sentence);
		expect(deviceOnlyText(t)).toBe(sentence);
	});

	test("a bot: connected, on its way, or why it isn't connected", () => {
		const bot = (where: ScheduleWhere) =>
			scheduleWhereText(t, where, NAMES, "bot");
		expect(bot({ fact: "hub" })).toBe("No device runs it.");
		expect(
			bot({
				fact: "device",
				bot: {
					event_id: "evt_helper",
					provider: "telegram",
					state: "connected",
					hold: null,
				},
				live: true,
				...ASSIGNED,
			}),
		).toBe("Connected from edge-berlin-01.");
		expect(bot({ fact: "released", ...ASSIGNED })).toBe(
			"Moves to edge-berlin-01 › invoice-reconcile when that service starts it.",
		);
		expect(bot({ fact: "returning", hubResumesAt: 9 })).toBe(
			"Taken back from its device. Nothing runs it after @9.",
		);
		const lead =
			"Assigned to edge-berlin-01 › invoice-reconcile, but it isn't connected: ";
		const idle = (extra: object) =>
			bot({ fact: "device_idle", why: "not_connected", ...ASSIGNED, ...extra });
		expect(idle({ botState: "token_refused" })).toBe(
			`${lead}its token was refused.`,
		);
		expect(idle({ botState: "webhook_set" })).toBe(
			`${lead}Telegram sends its messages to a webhook.`,
		);
		expect(idle({ why: "held", hold: "hub_too_old" })).toBe(
			`${lead}this hub can't hand bots to devices yet.`,
		);
		expect(idle({ why: "stopped" })).toBe(`${lead}the service is stopped.`);
	});

	test("a one-time schedule's own rows: when it runs, and how it ended", () => {
		const names = { device: "edge-berlin-01", own: "shop" };
		const lines = (run: ScheduleRun) => scheduleRunLines(t, run, names, TIME);
		expect(
			lines({
				state: "armed",
				entry: { ...ONCE, once_state: "pending" },
				next: { at: ONCE.once_at ?? 0, computed: false },
			})[0],
		).toStartWith("Runs once ");
		expect(
			lines({
				state: "armed",
				entry: { ...ONCE, once_state: "started", running: true },
				next: null,
			}),
		).toEqual(["A run is going now."]);
		const ended = (
			state: "ran" | "missed" | "passed",
			outcome?: "succeeded" | "cancelled",
		) =>
			lines({
				state: "finished",
				entry: { ...ONCE, once_state: state },
				finished: {
					state,
					at: ONCE.once_at ?? 0,
					...(outcome ? { outcome } : {}),
				},
			});
		expect(ended("ran", "succeeded")[0]).toMatch(/^Ran .+ · succeeded$/);
		expect(ended("ran", "cancelled")[0]).toMatch(
			/^Was cut off .+; it isn't started again\.$/,
		);
		expect(ended("missed")[0]).toMatch(
			/^Missed: edge-berlin-01 wasn't running at its time \(.+\)\.$/,
		);
		expect(ended("passed")[0]).toMatch(
			/^Its time \(.+\) had passed when shop could first run it\.$/,
		);
		expect(ended("passed")[1]).toBe("Nothing more to run.");
	});
});

describe("where a schedule of an online app runs, as one sentence", () => {
	test("“Runs on {device}” only for a schedule a running service is known to run", () => {
		const facts: ScheduleWhere[] = [
			{ fact: "hub" },
			{ fact: "released", ...ASSIGNED },
			{ fact: "returning", hubResumesAt: 9 },
			{ fact: "device_idle", why: "stopped", ...ASSIGNED },
			{
				fact: "device_unconfirmed",
				stale: false,
				readable: false,
				...ASSIGNED,
			},
		];
		for (const fact of facts) expect(say(fact)).not.toContain("Runs on edge");
		expect(
			say({ fact: "device", schedule: ENTRY, live: true, ...ASSIGNED }),
		).toBe("Runs on edge-berlin-01, not on the hub.");
	});

	test("assigned and idle: stopped, failing, removed, ended access, and every hold", () => {
		const idle = (
			why: Extract<ScheduleWhere, { fact: "device_idle" }>["why"],
			extra: object = {},
		) => say({ fact: "device_idle", why, ...ASSIGNED, ...extra });
		const lead =
			"Assigned to edge-berlin-01 › invoice-reconcile, but nothing runs it: ";
		expect(idle("stopped")).toBe(`${lead}the service is stopped.`);
		expect(idle("failed")).toBe(`${lead}the service keeps failing.`);
		expect(idle("removed")).toBe(`${lead}the service was removed.`);
		expect(idle("access_ended", { endedAt: 7 })).toBe(
			`${lead}its cloud access ended @7; the hub takes it back at its next scheduled time.`,
		);
		const holds = SCHEDULE_HOLDS.map((hold) => idle("held", { hold }));
		expect(new Set(holds).size).toBe(SCHEDULE_HOLDS.length);
		for (const sentence of holds) {
			expect(sentence.startsWith(lead)).toBe(true);
			expect(sentence).not.toMatch(MACHINE_WORDS);
		}
	});

	test("assigned and unknown: locked, readable without an answer, quiet for two hours, or hidden", () => {
		const unconfirmed = (extra: object) =>
			say({
				fact: "device_unconfirmed",
				stale: false,
				readable: false,
				...ASSIGNED,
				...extra,
			});
		expect(unconfirmed({})).toBe(
			"Assigned to edge-berlin-01. It last confirmed 2 ago; unlock the device to see whether it runs.",
		);
		expect(unconfirmed({ readable: true })).toBe(
			"Assigned to edge-berlin-01. It last confirmed 2 ago; its status doesn't say whether it runs.",
		);
		expect(unconfirmed({ stale: true })).toBe(
			"Assigned to edge-berlin-01, which has not confirmed since @2. It may be off.",
		);
		expect(
			say({
				fact: "device_unconfirmed",
				stale: true,
				readable: false,
				since: 1,
				seenAt: 2,
			}),
		).toBe(
			"Assigned to a device you can't see, not the hub. It last confirmed 2 ago.",
		);
	});

	test("released, also to a device the viewer can't see, and returning", () => {
		expect(say({ fact: "released", ...ASSIGNED })).toBe(
			"Moves to edge-berlin-01 › invoice-reconcile when that service starts it. The hub runs it until then.",
		);
		expect(say({ fact: "released", since: 1 })).toBe(
			"Moves to a device you can't see when its service starts it. The hub runs it until then.",
		);
		expect(say({ fact: "returning", hubResumesAt: 9 })).toBe(
			"Returns to the hub at @9.",
		);
		expect(say({ fact: "hub" })).toBe("Runs on the hub.");
	});
});

test("the way back to the hub exists for a release and for every assignment, not for the hub or a return", () => {
	const can = (where: ScheduleWhere | undefined) => canRunOnHubAgain(where);
	expect(can(undefined)).toBe(false);
	expect(can({ fact: "hub" })).toBe(false);
	expect(can({ fact: "device_only" })).toBe(false);
	expect(can({ fact: "returning", hubResumesAt: 9 })).toBe(false);
	expect(can({ fact: "released", since: 1 })).toBe(true);
	expect(can({ fact: "device_idle", why: "removed", ...ASSIGNED })).toBe(true);
	expect(
		can({
			fact: "device_unconfirmed",
			stale: false,
			readable: true,
			...ASSIGNED,
		}),
	).toBe(true);
	expect(
		can({ fact: "device", schedule: ENTRY, live: false, ...ASSIGNED }),
	).toBe(true);
});

describe("when a schedule runs, in words", () => {
	test("a sentence when one fits, the expression itself when none does", () => {
		expect(scheduleWords(t, "0 0 2 * * *")).toBe("At 02:00 every day");
		expect(scheduleWordsInline(t, "0 0 2 * * *")).toBe("at 02:00 every day");
		expect(scheduleWords(t, "0 0 6,14,22 * * *")).toBe(
			"Schedule 0 0 6,14,22 * * *",
		);
		expect(scheduleWordsInline(t, "0 0 6,14,22 * * *")).toBe(
			"by the schedule 0 0 6,14,22 * * *",
		);
	});

	test("an event without a time zone runs in UTC on a device, and the head says so", () => {
		const unset = {
			expression: "0 0 * * * *",
			timezone: "UTC",
			zoneSet: false,
		};
		expect(scheduleZone(t, unset)).toBe("UTC (the event sets no time zone)");
		expect(scheduleZone(t, { ...unset, zoneSet: true })).toBe("UTC");
		// A schedule a device reported carries no such flag: its zone is what it runs in.
		expect(scheduleZone(t, ENTRY)).toBe("Europe/Berlin");
		expect(scheduleHead(t, unset)).toBe(
			"Schedule · At :00 past every hour · UTC (the event sets no time zone)",
		);
	});

	test("a time reads in the schedule's zone; the reader's own time only when it differs", () => {
		const next = NOW0 + 12 * 3600;
		expect(scheduleTime(t, next, "Europe/Berlin", TIME)).toBe(
			"Oct 1 at 02:00 GMT+2 · in 12 hr. · Oct 1 at 00:00 your time",
		);
		// The tests run in UTC: the same clock, so nothing is repeated.
		expect(scheduleTime(t, next, "UTC", TIME)).toBe(
			"Oct 1 at 00:00 UTC · in 12 hr.",
		);
		// A zone this browser does not know: the reader's own time is all it can say.
		expect(scheduleTime(t, next, "Mars/Olympus", TIME)).toBe(
			"Oct 1 at 00:00 your time · in 12 hr.",
		);
	});
});

describe("schedules by name", () => {
	test("one list in the reader's language; an event without a known name keeps its id", () => {
		const names = new Map([
			["evt_a", "Nightly report"],
			["evt_b", "Weekly digest"],
		]);
		expect(scheduleNameList(["evt_a"], names, "en")).toBe("Nightly report");
		expect(scheduleNameList(["evt_a", "evt_b", "evt_c"], names, "en")).toBe(
			"Nightly report, Weekly digest, and evt_c",
		);
		expect(scheduleNameList(["evt_a", "evt_b"], names, "de")).toBe(
			"Nightly report und Weekly digest",
		);
	});
});

describe("what a schedule of a service does now", () => {
	const names = { device: "edge-berlin-01" };
	const lines = (run: ScheduleRun) => scheduleRunLines(t, run, names, TIME);
	const armed = (
		entry: Partial<ServiceSchedule>,
		next: Extract<ScheduleRun, { state: "armed" }>["next"],
	): ScheduleRun => ({ state: "armed", entry: { ...ENTRY, ...entry }, next });

	test("armed: next and last run, a run in progress, and a clock that is behind", () => {
		const at = NOW0 + 12 * 3600;
		expect(lines(armed({}, { at, computed: false }))).toEqual([
			"Next run Oct 1 at 02:00 GMT+2 · in 12 hr. · Oct 1 at 00:00 your time",
		]);
		expect(lines(armed({}, { at, computed: true }))).toEqual([
			"Next run by its schedule: Oct 1 at 02:00 GMT+2 · in 12 hr. · Oct 1 at 00:00 your time",
		]);
		expect(
			lines(
				armed(
					{
						running: true,
						last_at: NOW0 - 12 * 3600,
						last_outcome: "failed",
					},
					null,
				),
			),
		).toEqual([
			"A run is going now.",
			"Last run 02:00 GMT+2 · 12 hr. ago · 00:00 your time · failed",
		]);
		// A status snapshot keeps the result and not the time.
		expect(lines(armed({ last_outcome: "succeeded" }, null))).toEqual([
			"Last run succeeded",
		]);
		// The service's own page says why no next run is shown; a table cell stays short.
		const page = (run: ScheduleRun) =>
			scheduleRunLines(t, run, names, TIME, { noNext: true });
		expect(page(armed({ last_outcome: "succeeded" }, null))).toEqual([
			"No further run is scheduled.",
			"Last run succeeded",
		]);
		expect(
			page({ state: "armed", entry: ENTRY, next: null, stale: true }),
		).toEqual(["This status is too old to say when it runs next."]);
		expect(page(armed({}, { at, computed: false }))).toEqual(
			lines(armed({}, { at, computed: false })),
		);
		expect(
			lines(armed({ clock_behind: true }, { at, computed: false }))[0],
		).toBe(
			"Waiting until Oct 1 at 02:00 GMT+2 · in 12 hr. · Oct 1 at 00:00 your time: edge-berlin-01's clock is behind its last run.",
		);
	});

	test("held, stopped, not reported, an agent too old to say, and unknown each have their own sentence", () => {
		const said = [
			...SCHEDULE_HOLDS.map((hold) =>
				lines({ state: "held", entry: { ...ENTRY, hold }, hold }),
			),
			lines({ state: "stopped" }),
			lines({ state: "not_reported" }),
			lines({ state: "needs_agent" }),
			lines({ state: "unknown" }),
		];
		expect(new Set(said.map((value) => value.join("|"))).size).toBe(
			said.length,
		);
		for (const [sentence, ...rest] of said) {
			expect(rest).toEqual([]);
			expect(sentence).not.toMatch(MACHINE_WORDS);
			expect(sentence).not.toContain("Next run");
		}
		expect(lines({ state: "stopped" })).toEqual([
			"No runs while the service is not running.",
		]);
	});

	test("a hold names who runs it instead, when that is known", () => {
		expect(scheduleHoldText(t, "other_service", { service: "crm-sync" })).toBe(
			"Not running here: crm-sync on this device runs it.",
		);
		expect(scheduleHoldText(t, "other_service")).toBe(
			"Not running here: another service on this device runs it.",
		);
		expect(
			scheduleHoldText(t, "runs_elsewhere", {
				where: "studio-mac-mini › invoice-reports",
			}),
		).toBe(
			"Not running here: studio-mac-mini › invoice-reports runs it. A schedule runs in one place.",
		);
		const where: ScheduleWhere = {
			fact: "device_unconfirmed",
			stale: false,
			readable: false,
			since: 1,
			seenAt: 2,
			deviceId: "dev-studio",
			serviceId: "invoice-reports",
		};
		const base = {
			deviceId: "dev-edge",
			serviceId: "invoice-extractor",
			device: "edge-berlin-01",
			eventId: ENTRY.event_id,
		};
		expect(
			scheduleRunNames(t, {
				...base,
				where,
				deviceName: () => "studio-mac-mini",
				siblings: [
					{ serviceId: "invoice-extractor", schedules: [ENTRY] },
					{ serviceId: "invoice-nightly", schedules: [ENTRY] },
					{
						serviceId: "held",
						schedules: [{ ...ENTRY, hold: "other_service" }],
					},
				],
			}),
		).toEqual({
			device: "edge-berlin-01",
			own: "invoice-extractor",
			service: "invoice-nightly",
			where: "studio-mac-mini › invoice-reports",
		});
		// The hub names this very service: there is no other place to name.
		expect(
			scheduleRunNames(t, {
				...base,
				where: {
					...where,
					deviceId: "dev-edge",
					serviceId: "invoice-extractor",
				},
			}),
		).toEqual({ device: "edge-berlin-01", own: "invoice-extractor" });
	});

	test("every outcome and every skip reason has words", () => {
		for (const outcome of SCHEDULE_OUTCOMES)
			expect(scheduleOutcome(t, outcome)).not.toMatch(MACHINE_WORDS);
		for (const reason of SCHEDULE_SKIP_REASONS)
			expect(scheduleSkip(t, reason)).not.toMatch(MACHINE_WORDS);
	});
});
