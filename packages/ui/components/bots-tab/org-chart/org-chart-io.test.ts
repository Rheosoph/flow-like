import { describe, expect, it } from "bun:test";
import type {
	IOrgBot,
	IOrgBotInputs,
	ITeam,
} from "../../../state/backend-state/bot-team-state";
import {
	botPorts,
	calendarMark,
	hasPorts,
	teamFlow,
	teamHasPorts,
} from "./org-chart-io";

const noInputs: IOrgBotInputs = {
	mail: null,
	webhooks: [],
	own_calendar: false,
	connected_calendars: [],
};

function bot(id: string, inputs?: Partial<IOrgBotInputs>): IOrgBot {
	return {
		id,
		owner_id: "mara",
		name: id,
		appearance: {} as IOrgBot["appearance"],
		location: "server",
		inputs: inputs ? { ...noInputs, ...inputs } : undefined,
	};
}

function team(bots: [string, "active" | "pending"][]): ITeam {
	return {
		id: "finance",
		name: "Finance",
		direct_asks: false,
		people: [],
		bots: bots.map(([bot_id, status]) => ({
			bot_id,
			owner_id: "mara",
			status,
			direct_agreed: false,
		})),
		created_at: "",
		updated_at: "",
	};
}

const mail = { address: "ledger@mail.test", received: 12, sent: 4 };
const hooks = ["Stripe", "Invoices", "Payroll"].map((name) => ({
	name,
	last_called_at: null,
}));

describe("botPorts", () => {
	it("puts the mail address first as an input and its sent mail as the output", () => {
		const ports = botPorts(
			bot("ledger", { mail, webhooks: hooks.slice(0, 1) }),
		);
		expect(ports.inputs.map((port) => port.kind)).toEqual(["mail", "webhook"]);
		expect(ports.outputs).toEqual([
			{ kind: "mail", key: "mail-out", address: mail.address, sent: 4 },
		]);
		expect(ports.moreInputs).toBe(0);
	});

	it("folds inputs past the row limit into a count", () => {
		const ports = botPorts(bot("ledger", { mail, webhooks: hooks }));
		expect(ports.inputs).toHaveLength(2);
		expect(ports.moreInputs).toBe(2);
	});

	it("shows exactly the limit without folding", () => {
		const ports = botPorts(bot("ledger", { webhooks: hooks }));
		expect(ports.inputs).toHaveLength(3);
		expect(ports.moreInputs).toBe(0);
		expect(ports.outputs).toEqual([]);
	});

	it("has nothing for a bot from an older hub or without ways in", () => {
		expect(botPorts(bot("old"))).toEqual({
			inputs: [],
			moreInputs: 0,
			outputs: [],
		});
		expect(hasPorts(bot("quiet", { own_calendar: true }))).toBe(false);
	});
});

describe("calendarMark", () => {
	it("tells its own calendar from connected ones", () => {
		expect(calendarMark(bot("own", { own_calendar: true }))).toEqual({
			own: true,
			connected: [],
		});
		expect(
			calendarMark(
				bot("both", {
					own_calendar: true,
					connected_calendars: ["google", "caldav"],
				}),
			),
		).toEqual({ own: true, connected: ["google", "caldav"] });
		expect(calendarMark(bot("none", {}))).toBeNull();
		expect(calendarMark(bot("old"))).toBeNull();
	});
});

describe("teamFlow", () => {
	const bots = new Map([
		["ledger", bot("ledger", { mail, webhooks: hooks.slice(0, 2) })],
		[
			"clerk",
			bot("clerk", {
				mail: { address: "clerk@mail.test", received: 3, sent: 1 },
				connected_calendars: ["microsoft"],
			}),
		],
		["invited", bot("invited", { mail, webhooks: hooks })],
	]);
	const botOf = (id: string) => bots.get(id);

	it("adds up the active bots only", () => {
		expect(
			teamFlow(
				team([
					["ledger", "active"],
					["clerk", "active"],
					["invited", "pending"],
				]),
				botOf,
			),
		).toEqual({
			addresses: 2,
			webhooks: 2,
			received: 15,
			sent: 5,
			calendars: 1,
		});
	});

	it("gives rails only to teams with an active bot that has a way in", () => {
		expect(teamHasPorts(team([["invited", "pending"]]), botOf)).toBe(false);
		expect(teamHasPorts(team([["clerk", "active"]]), botOf)).toBe(true);
	});
});
