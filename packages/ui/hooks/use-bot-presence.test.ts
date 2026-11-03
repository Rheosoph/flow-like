import { describe, expect, test } from "bun:test";
import type { IBot } from "../state/backend-state/bot-state";
import { botPresence } from "./use-bot-presence";

function bot(overrides: Partial<IBot>): IBot {
	return {
		id: "bot_a",
		system: null,
		location: "desktop",
		...overrides,
	} as IBot;
}

describe("a bot's presence", () => {
	test("a desktop or device bot is what the hub reported", () => {
		const reports = {
			bot_a: { state: "offline", last_seen_at: "2026-11-03T10:00:00Z" },
		} as const;
		expect(botPresence(bot({}), reports)).toEqual(reports.bot_a);
		expect(botPresence(bot({ location: "device" }), reports)).toEqual(
			reports.bot_a,
		);
	});

	test("a hosted bot nobody reported, or before the reports arrive, is unknown", () => {
		expect(botPresence(bot({}), {})).toEqual({ state: "unknown" });
		expect(botPresence(bot({}), undefined)).toEqual({ state: "unknown" });
	});

	test("server and built-in bots are online whatever the reports say", () => {
		const offline = { bot_a: { state: "offline" } } as const;
		expect(botPresence(bot({ location: "server" }), offline)).toEqual({
			state: "online",
		});
		expect(botPresence(bot({ system: "general" }), offline)).toEqual({
			state: "online",
		});
	});

	test("a bot on this computer and a missing bot have none", () => {
		expect(
			botPresence(bot({ local: true }), { bot_a: { state: "online" } }),
		).toBe(null);
		expect(botPresence(undefined, {})).toBeNull();
	});
});
