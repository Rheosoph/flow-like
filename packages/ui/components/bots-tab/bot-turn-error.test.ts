import { describe, expect, test } from "vitest";
import { botTurnError } from "./bot-turn-error";

const TEXT =
	"I can't answer yet: This profile has no language model yet. Add one to get an answer.";

describe("the card of a turn a bot could not answer", () => {
	test("the owner of a bot without a model gets the picker for cloud models", () => {
		const error = botTurnError("BOT_TURN_NO_MODEL", TEXT, {
			title: "Couldn't answer",
			ownBot: true,
		});

		expect(error.action).toEqual({
			kind: "add-model",
			label: "Choose a model",
			hostedOnly: true,
		});
		expect(error.title).toBe("Couldn't answer");
		expect(error.message).toBe(TEXT);
		expect(error.code).toBe("BOT_TURN_NO_MODEL");
	});

	test("the card names the bot, so the pick goes to the profile that bot reads", () => {
		const error = botTurnError("BOT_TURN_NO_MODEL", TEXT, {
			title: "",
			ownBot: true,
			botId: "bot_1",
		});

		expect(error.action).toMatchObject({ kind: "add-model", botId: "bot_1" });
	});

	test("someone else's bot only says why: their profile is not the one to fix", () => {
		const error = botTurnError("BOT_TURN_NO_MODEL", TEXT, {
			title: "Couldn't answer",
			ownBot: false,
		});

		expect(error.action).toBeUndefined();
		expect(error.kind).toBe("config");
		expect(error.message).toBe(TEXT);
	});

	test("a reply stored before the code existed is read by its sentence, without the blame on the plan", () => {
		const stored =
			"I can't answer right now: None of the models in this profile are included in your plan. Upgrade, or add a model your plan covers in Settings → Models.";
		const owner = botTurnError("BOT_TURN_QUOTA", stored, {
			title: "",
			ownBot: true,
		});
		const visitor = botTurnError("BOT_TURN_QUOTA", stored, {
			title: "",
			ownBot: false,
		});

		expect(owner.action?.kind).toBe("add-model");
		expect(visitor.action).toBeUndefined();
		for (const error of [owner, visitor]) {
			expect(error.kind).toBe("config");
			expect(error.message).toBe(
				"No language model in this profile can answer here.",
			);
		}
	});

	test("a bot this desktop runs may answer with a model on the device", () => {
		const error = botTurnError(
			"BOT_TURN_SETUP",
			"Bot 'Scout' can't answer: no language model it can use runs on this desktop (No available model found in this profile). Add one in Settings → Models or pick one in the bot's settings.",
			{ title: "", ownBot: true },
		);

		expect(error.action).toMatchObject({
			kind: "add-model",
			hostedOnly: false,
		});
		expect(error.message).not.toContain("Settings");
	});

	test("other failures keep their kind and offer nothing", () => {
		const quota = botTurnError("BOT_TURN_QUOTA", "Plan used up", {
			title: "",
			ownBot: true,
		});
		const unknown = botTurnError("BOT_TURN_FUTURE", "Something new", {
			title: "",
			ownBot: true,
		});

		expect([quota.kind, quota.action]).toEqual(["billing", undefined]);
		expect(unknown.kind).toBe("generic");
	});
});
