import { describe, expect, test } from "bun:test";
import {
	NO_HIDDEN_BOTS,
	botVisibilityKey,
	shownBots,
	withBotHidden,
} from "./bot-visibility-model";

const assistantHere = { id: "local-assistant", system: "general" as const };
const assistantInAccount = {
	id: "account-assistant",
	system: "general" as const,
};
const researcher = { id: "bot-research", system: null };

describe("bot visibility in a profile", () => {
	test("names built-in bots by system key so every row of one hides together", () => {
		expect(botVisibilityKey(assistantHere)).toBe("general");
		expect(botVisibilityKey(assistantInAccount)).toBe("general");
		expect(botVisibilityKey(researcher)).toBe("bot-research");
		expect(
			shownBots(
				[assistantHere, assistantInAccount, researcher],
				new Set(["general"]),
			),
		).toEqual([researcher]);
	});

	test("shows every bot while nothing is hidden", () => {
		const bots = [assistantHere, researcher];
		expect(shownBots(bots, NO_HIDDEN_BOTS)).toBe(bots);
	});

	test("hides and shows one bot without touching the entries of others", () => {
		const elsewhere = "bot-on-another-device";
		const hidden = withBotHidden([elsewhere], "bot-research", true);
		expect(hidden).toEqual([elsewhere, "bot-research"]);
		expect(withBotHidden(hidden, "bot-research", true)).toEqual(hidden);
		expect(withBotHidden(hidden, "bot-research", false)).toEqual([elsewhere]);
		expect(withBotHidden([elsewhere], "bot-research", false)).toEqual([
			elsewhere,
		]);
	});
});
