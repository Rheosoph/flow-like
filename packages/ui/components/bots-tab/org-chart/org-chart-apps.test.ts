import { describe, expect, test } from "bun:test";
import type {
	IOrgApp,
	IOrgView,
	ITeam,
} from "../../../state/backend-state/bot-team-state";
import {
	appFamily,
	appLinks,
	canUnpin,
	matchesFilter,
	scheduleOf,
	teamDeck,
	triggerKinds,
} from "./org-chart-apps";

const appearance = {} as never;

function app(id: string, app_type: IOrgApp["app_type"], name = id): IOrgApp {
	return {
		id,
		name,
		app_type,
		can_open: true,
		can_request: false,
		reader_admin: false,
		updated_at: "",
	};
}

const team: ITeam = {
	id: "finance",
	name: "Finance",
	direct_asks: false,
	people: [
		{ user_id: "maya", role: "admin", status: "active" },
		{ user_id: "ben", role: "member", status: "active" },
	],
	bots: [
		{
			bot_id: "ledger",
			owner_id: "maya",
			status: "active",
			direct_agreed: false,
		},
		{
			bot_id: "auditor",
			owner_id: "ben",
			status: "pending",
			direct_agreed: false,
		},
	],
	apps: [
		{ app_id: "close", status: "active", pinned_by: "maya", pinned_at: "" },
		{ app_id: "db", status: "pending", pinned_by: "ben", pinned_at: "" },
	],
	created_at: "",
	updated_at: "",
};

const org: IOrgView = {
	teams: [team],
	bots: [
		{
			id: "ledger",
			owner_id: "maya",
			appearance,
			location: "server",
			app_ids: ["close", "agent", "form"],
		},
		{
			id: "auditor",
			owner_id: "ben",
			appearance,
			location: "server",
			app_ids: ["spend"],
		},
	],
	invitations: [],
	apps: [
		app("close", "DataPipeline", "Month-end close"),
		app("db", "DataFocus", "Ledger DB"),
		app("agent", "Agent", "Inbox agent"),
		app("form", "Form", "Expenses"),
		app("spend", "Analytics", "Spend"),
	],
};

describe("appFamily", () => {
	test("groups the six types into four families", () => {
		expect(appFamily("Agent")).toBe("agents");
		expect(appFamily("CustomInterface")).toBe("apps");
		expect(appFamily("Form")).toBe("apps");
		expect(appFamily("DataFocus")).toBe("data");
		expect(appFamily("Analytics")).toBe("data");
		expect(appFamily("DataPipeline")).toBe("automations");
		expect(appFamily(null)).toBe("other");
		expect(matchesFilter(app("x", null), "all")).toBe(true);
		expect(matchesFilter(app("x", null), "data")).toBe(false);
	});
});

describe("teamDeck", () => {
	test("pins and the active bots' apps, by family, never a pending bot's apps", () => {
		const deck = teamDeck(org, team);
		expect(deck.map((entry) => entry.app.id)).toEqual([
			"agent",
			"form",
			"db",
			"close",
		]);
		const agent = deck[0];
		expect(agent.pin).toBeNull();
		expect(agent.usedBy).toEqual(["ledger"]);
		expect(deck.find((entry) => entry.app.id === "db")?.pin?.status).toBe(
			"pending",
		);
	});
});

describe("appLinks", () => {
	test("an app lights the bots that use it and the teams it belongs to", () => {
		const links = appLinks(org, "close");
		expect([...links.botIds]).toEqual(["ledger"]);
		expect([...links.teamIds]).toEqual(["finance"]);
	});
});

describe("canUnpin", () => {
	const pin = {
		app_id: "db",
		status: "pending" as const,
		pinned_by: "ben",
		pinned_at: "",
	};
	test("team admins, the pinner and the app's admins", () => {
		expect(canUnpin(team, pin, { reader_admin: false }, "maya")).toBe(true);
		expect(canUnpin(team, pin, { reader_admin: false }, "ben")).toBe(true);
		expect(canUnpin(team, pin, { reader_admin: true }, "zoe")).toBe(true);
		expect(canUnpin(team, pin, { reader_admin: false }, "zoe")).toBe(false);
		expect(canUnpin(team, pin, { reader_admin: true }, null)).toBe(false);
	});
});

describe("scheduleOf", () => {
	test("reads common cron shapes, with or without seconds", () => {
		expect(scheduleOf("*/15 * * * *")).toEqual({ kind: "minutes", every: 15 });
		expect(scheduleOf("0 */15 * * * *")).toEqual({
			kind: "minutes",
			every: 15,
		});
		expect(scheduleOf("5 * * * *")).toEqual({ kind: "hourly" });
		expect(scheduleOf("0 */6 * * *")).toEqual({ kind: "hours", every: 6 });
		expect(scheduleOf("0 2 * * *")).toEqual({ kind: "daily", time: "02:00" });
		expect(scheduleOf("0 0 9 * * MON")).toEqual({
			kind: "cron",
			expression: "0 0 9 * * MON",
		});
	});
});

describe("triggerKinds", () => {
	test("maps event types to kinds once, ignoring unknown ones", () => {
		expect(
			triggerKinds(["email", "cron", "simple_chat", "api", "rest", "page"]),
		).toEqual(["chat", "mail", "schedule", "webhook"]);
	});
});
