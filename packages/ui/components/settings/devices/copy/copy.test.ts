import { describe, expect, test } from "bun:test";
import { getI18n } from "@flow-like/locales";
import {
	EVENT_INELIGIBLE_CODES,
	type EventIneligibleCode,
} from "../../../../lib/device-management/deployment";
import type {
	GateFailure,
	GateReason,
} from "../../../../lib/device-management/model/types";
import type { PreflightRow } from "../../../../lib/device-management/workspace/types";
import type { DevicesT } from "../primitives/area-context";
import { appCopy } from "./app-copy";
import {
	type AttentionCopyItem,
	attentionCopy,
	attentionNames,
	defaultCopyTime,
} from "./attention-copy";
import {
	agentTooOldCopy,
	cantHereCopy,
	eligibilityCopy,
	eventRunsCopy,
	eventTypeLabel,
	howItRunsCopy,
} from "./eligibility-copy";
import { gateCopy } from "./gate-copy";
import { headlineNames } from "./headline-copy";
import { preflightCopy } from "./preflight-copy";

const t = getI18n().getFixedT("en", "devices") as DevicesT;
const time = defaultCopyTime();
const SOON = Math.floor(time.now / 1000) + 6 * 3600;

function item(
	code: AttentionCopyItem["copy"]["code"],
	params: AttentionCopyItem["copy"]["params"],
): AttentionCopyItem {
	return {
		key: code,
		copy: { code, params },
		lastKnown: false,
		firstSeenAt: SOON,
	};
}

describe("attention copy", () => {
	const names = new Map([["usr_mira", "Mira Novak"]]);
	const ctx = { time, personName: (id: string) => names.get(id) };

	test("an account id is never the name of a person (R3, R15)", () => {
		const grant = (person: string) =>
			attentionCopy(
				t,
				item("grant_expiring", {
					device: "edge-berlin-01",
					person,
					expiresAt: SOON,
				}),
				ctx,
			).sentence;
		expect(grant("usr_mira")).toStartWith(
			"Mira Novak's access to edge-berlin-01 ends ",
		);
		expect(grant("usr_9QmT3rVb")).toStartWith(
			"One person's access to edge-berlin-01 ends ",
		);
		expect(grant("usr_9QmT3rVb")).not.toContain("usr_");

		const sandbox = attentionCopy(
			t,
			item("code_running_access_without_sandbox", {
				device: "edge-berlin-01",
				person: "usr_9QmT3rVb",
			}),
			ctx,
		).sentence;
		expect(sandbox).toBe(
			"One person can run code on edge-berlin-01 with the agent's full access.",
		);

		const pending = (owner: string) =>
			attentionCopy(
				t,
				item("access_request_pending", { device: "lab-gpu-02", owner }),
				ctx,
			).sentence;
		expect(pending("usr_mira")).toBe(
			"Waiting for Mira Novak to approve your access to lab-gpu-02.",
		);
		expect(pending("usr_9QmT3rVb")).toBe(
			"Waiting for the owner to approve your access to lab-gpu-02.",
		);
	});

	test("an agent that can't take the hub's release list is told why, instead of being offered the update", () => {
		const update = (extra: Record<string, number>) =>
			attentionCopy(
				t,
				item("agent_update_available", {
					device: "edge-berlin-01",
					available: "0.2.0",
					running: "0.1.0",
					...extra,
				}),
				ctx,
			).sentence;
		expect(update({ needsShortRelease: 1 })).toBe(
			"Agent 0.2.0 is available for edge-berlin-01, but its agent (0.1.0) only accepts releases valid for 30 days or less. Ask the hub operator to renew the release for 30 days, or set this device up again.",
		);
		expect(update({})).toBe(
			"Agent 0.2.0 is available for edge-berlin-01 (running 0.1.0).",
		);
	});

	test("a schedule a running service holds says why; an unknown reason is never its wire value", () => {
		const held = (params: Record<string, string | number>) =>
			attentionCopy(
				t,
				item("schedule_held", {
					service: "invoice-extractor",
					device: "edge-berlin-01",
					...params,
				}),
				ctx,
			).sentence;
		expect(held({ hold: "not_released", held: 1 })).toBe(
			"A schedule of invoice-extractor on edge-berlin-01 isn't running there: it was not moved to this service.",
		);
		expect(held({ hold: "runs_elsewhere", held: 2 })).toBe(
			"Some schedules of invoice-extractor on edge-berlin-01 aren't running there: another service runs it.",
		);
		expect(held({ hold: "paused_by_a_newer_agent", held: 1 })).toBe(
			"A schedule of invoice-extractor on edge-berlin-01 isn't running there: reason unknown.",
		);
		expect(
			attentionCopy(
				t,
				item("schedule_failed", {
					service: "invoice-extractor",
					device: "edge-berlin-01",
					failed: 1,
				}),
				ctx,
			).sentence,
		).toBe(
			"The last run of a schedule of invoice-extractor on edge-berlin-01 failed. The service keeps running.",
		);
	});

	test("a bot that isn't connected, is refused or used elsewhere says why; a missed one-time schedule says when", () => {
		const say = (
			code: AttentionCopyItem["copy"]["code"],
			params: Record<string, string | number>,
		) =>
			attentionCopy(
				t,
				item(code, {
					service: "shop",
					device: "edge-berlin-01",
					...params,
				}),
				ctx,
			).sentence;
		expect(say("bot_held", { hold: "hub_too_old", held: 1 })).toBe(
			"A bot of shop on edge-berlin-01 isn't connected: this hub can't hand bots to devices yet.",
		);
		expect(say("bot_held", { hold: "runs_elsewhere", held: 2 })).toBe(
			"Some bots of shop on edge-berlin-01 aren't connected: another service runs it.",
		);
		expect(say("bot_token_refused", { provider: "telegram" })).toBe(
			"Telegram refused the token of a bot of shop on edge-berlin-01. Enter a new one under Configuration.",
		);
		expect(say("bot_token_refused", { provider: "matrix" })).toStartWith(
			"The provider refused",
		);
		expect(say("bot_intents_refused", { provider: "discord" })).toBe(
			"Discord refused the permissions of a bot of shop on edge-berlin-01. Turn on the message content intent in the Discord Developer Portal, then restart shop.",
		);
		expect(say("bot_conflict", { state: "conflict" })).toBe(
			"Another program uses the token of a bot of shop on edge-berlin-01. A bot runs in one place: stop it there.",
		);
		expect(say("bot_conflict", { state: "webhook_set" })).toBe(
			"Telegram sends the messages of a bot of shop on edge-berlin-01 to a webhook. Remove it in Events, then restart shop.",
		);
		expect(say("schedule_once_missed", { time: SOON })).toMatch(
			/^A one-time schedule of shop on edge-berlin-01 didn't run: edge-berlin-01 wasn't running at its time \(.+\)\.$/,
		);
	});

	test("an item names the device and service its sentence sets in mono", () => {
		expect(
			attentionNames(
				item("service_crash_looping", {
					device: "warehouse-pi",
					service: "scanner-ingest",
					ready: 0,
					requested: 1,
				}),
			),
		).toEqual(["warehouse-pi", "scanner-ingest"]);
		expect(attentionNames(item("stale_local_keys", {}))).toEqual([]);
	});
});

describe("eligibility copy", () => {
	test("every reason an event can't run has words, never its code or a placeholder", () => {
		for (const code of EVENT_INELIGIBLE_CODES) {
			const copy = eligibilityCopy(t, {
				code,
				eventType: "api",
				detail: "/ui/x",
			});
			for (const text of [copy.long, copy.short]) {
				expect(text).not.toMatch(/\b[a-z]+_[a-z_]+\b/);
				expect(text).not.toContain("{{");
				expect(text.length).toBeGreaterThan(3);
			}
		}
	});

	test("a route or bot setting a device can't read is named; an older hub says what it can't hand over", () => {
		const long = (
			code: EventIneligibleCode,
			eventType: string,
			detail?: string,
		) =>
			eligibilityCopy(t, { code, eventType, ...(detail ? { detail } : {}) })
				.long;
		expect(long("route_reserved", "api", "/ui/x")).toBe(
			"Its path /ui/x is used by the service itself. Choose another path in Events.",
		);
		expect(long("route_invalid", "http", "TRACE")).toBe(
			"A device can't serve its method or path: TRACE",
		);
		expect(long("bot_invalid", "telegram", "chat_whitelist")).toBe(
			"Its bot settings can't be read on a device: chat_whitelist",
		);
		expect(long("hub_type", "api")).toBe(
			"This hub can't hand Endpoints to devices yet. Update the hub.",
		);
		expect(long("hub_type", "generic_form")).toBe(
			"This hub can't hand forms and quick actions to devices yet. Update the hub.",
		);
		expect(long("hub_type", "discord")).toBe(
			"This hub can't hand bots to devices yet. Update the hub.",
		);
		expect(
			eligibilityCopy(t, { code: "schedule_once", eventType: "cron" }),
		).toEqual({
			long: "This device's agent can't run one-time schedules yet. Update the device agent.",
			short: "Agent too old",
			fix: null,
		});
	});

	test("an older agent is told which part it lacks", () => {
		expect(agentTooOldCopy(t, "edge", "api_events")).toEqual({
			long: "edge's agent is too old to serve Endpoints.",
			short: "Agent too old",
			fix: "Update the device agent to serve Endpoints",
		});
		expect(agentTooOldCopy(t, "edge", "telegram_bots").fix).toBe(
			"Update the device agent to run Telegram bots",
		);
		expect(agentTooOldCopy(t, "edge", "discord_bots").long).toBe(
			"edge's agent is too old to run Discord bots.",
		);
		expect(agentTooOldCopy(t, "edge", "on_demand_events").long).toBe(
			"edge's agent is too old to run forms and quick actions.",
		);
		expect(agentTooOldCopy(t, "edge", "scheduled_once").long).toBe(
			"edge's agent is too old to run one-time schedules.",
		);
		expect(agentTooOldCopy(t, "edge").long).toBe(
			"edge's agent is too old to run schedules.",
		);
		expect(
			cantHereCopy(t, { why: "agent", feature: "api_events" }, "edge"),
		).toBe("edge's agent is too old to serve Endpoints.");
		expect(cantHereCopy(t, { why: "runs_elsewhere", bot: true }, "edge")).toBe(
			"Another service runs this bot. A bot runs in one place.",
		);
	});

	test("type names and how each new kind runs", () => {
		expect(eventTypeLabel(t, "api")).toBe("Endpoint");
		expect(eventTypeLabel(t, "http")).toBe("Endpoint");
		expect(eventTypeLabel(t, "telegram")).toBe("Telegram bot");
		expect(eventTypeLabel(t, "discord")).toBe("Discord bot");
		expect(eventTypeLabel(t, "teams")).toBe("Teams bot");
		const runs = (rule: Parameters<typeof howItRunsCopy>[1]) =>
			howItRunsCopy(t, rule);
		const base = { hosted: false, readiness: "explicit" } as const;
		expect(runs({ ...base, kind: "on_demand" })).toBe(
			"Started by a person · from Devices or the service page",
		);
		expect(
			runs({
				...base,
				kind: "bot",
				bot: {
					provider: "discord",
					open: false,
					savedToken: false,
					prefix: "",
					mentions: true,
				},
			}),
		).toBe("Runs on its own · stays connected to Discord");
		const once = {
			date: "2026-10-15",
			time: "09:00",
			at: 1792047600,
			timezone: "Europe/Berlin",
			zoneSet: true,
		};
		expect(runs({ ...base, kind: "scheduled", once })).toBe(
			"Runs once · the device starts it",
		);
		expect(eventRunsCopy(t, { ...base, kind: "scheduled", once })).toBe(
			"Once on 2026-10-15 at 09:00 · Europe/Berlin",
		);
		expect(
			eventRunsCopy(t, {
				hosted: true,
				readiness: "listener",
				kind: "served",
				route: { method: "GET", path: "/orders" },
			}),
		).toBe("GET /orders · served by the device");
	});
});

describe("headline copy", () => {
	test("a headline names every device and service of its sentences once", () => {
		expect(
			headlineNames({
				code: "fleet.critical",
				params: { count: 2 },
				lists: { names: ["warehouse-pi", "edge-berlin-01"] },
				rest: [
					{
						code: "fleet.coverage",
						params: { readable: 3, active: 5, locked: 1 },
						lists: { locked: ["lab-gpu-02"], events: ["Nightly sync"] },
					},
					{
						code: "fleet.soon_later",
						params: { device: "edge-berlin-01", service: "support-bot" },
					},
				],
			}),
		).toEqual(["warehouse-pi", "edge-berlin-01", "lab-gpu-02", "support-bot"]);
	});
});

describe("gate copy", () => {
	const gate = (
		code: GateReason,
		params: GateFailure["copy"]["params"],
	): GateFailure => ({
		ok: false,
		gate: "G0",
		kind: "live",
		hide: false,
		copy: { code, params },
	});

	test("a live gate names what it gates when the caller says so", () => {
		const device = "warehouse-pi";
		expect(gateCopy(t, gate("offline_needs_live", { device })).inline).toBe(
			"warehouse-pi is offline. This needs a live connection.",
		);
		expect(
			gateCopy(t, gate("offline_needs_live", { device, action: "Start" }))
				.inline,
		).toBe("warehouse-pi is offline. Start needs a live connection.");
		expect(
			gateCopy(
				t,
				gate("offline_needs_live", {
					device,
					action: "Restart and Stop",
					actions: 2,
				}),
			).inline,
		).toBe("warehouse-pi is offline. Restart and Stop need a live connection.");
		expect(
			gateCopy(
				t,
				gate("never_connected_needs_live", { device, action: "Start" }),
			).inline,
		).toBe(
			"warehouse-pi hasn't checked in yet. Start needs a live connection.",
		);
	});
});

describe("app copy", () => {
	test("the newest version isn't called absent while a service's version can't be told", () => {
		const foot = {
			version: "v1.5.0",
			hash: "7c2d1e90",
			when: "today 13:00",
			mode: "online" as const,
			running: 0,
			total: 2,
		};
		const copy = appCopy(t);
		expect(copy.versionFoot(foot)).toContain("isn't running anywhere yet.");
		expect(copy.versionFoot({ ...foot, unknown: 1 })).toBe(
			"Newest version v1.5.0 (7c2d1e90), built today 13:00, isn't on any service whose version is known.",
		);
		expect(copy.versionFoot({ ...foot, running: 1, unknown: 1 })).toContain(
			"runs on 1 of 2 services.",
		);
	});
});

describe("preflight copy", () => {
	test("the clock row says which clocks agree (SPEC §3.10)", () => {
		const row: PreflightRow = {
			id: "D9",
			status: "pass",
			source: "hub",
			copy: { code: "clock_ok" },
		};
		expect(preflightCopy(t, row).text).toBe("This computer and the hub agree");
	});
});
