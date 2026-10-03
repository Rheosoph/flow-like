import { expect, test } from "bun:test";
import {
	DISCORD_INTENTS,
	botHandle,
	botTokenEventId,
	botTokenKey,
	botTokenProblem,
	botTokenVariable,
	deviceBot,
	isBotTokenKey,
	savedBotToken,
} from "./bot-config";

/* The bot settings rule of run-more-2-design §1.4; the bots crate's test carries the same cases. */

const TELEGRAM = {
	sink_type: "telegram",
	bot_token: "123456789:AAH-secret_token_value_123",
	bot_name: "Flow-Like Bot",
	bot_description: "",
	chat_whitelist: [],
	chat_blacklist: [],
	respond_to_mentions: true,
	respond_to_private: true,
	command_prefix: "/",
};
const DISCORD = {
	sink_type: "discord",
	token: "",
	bot_name: "Flow-Like Bot",
	bot_description: "",
	intents: ["Guilds", "GuildMessages", "MessageContent"],
	channel_whitelist: [],
	channel_blacklist: [],
	respond_to_mentions: true,
	respond_to_dms: true,
	command_prefix: "!",
};

test("the editor's default settings are read, with their effective values", () => {
	expect(deviceBot("telegram", TELEGRAM)).toEqual({
		ok: true,
		bot: {
			provider: "telegram",
			open: true,
			savedToken: true,
			prefix: "/",
			mentions: true,
		},
	});
	// A Discord bot reads no prefix, like the desktop app's: the saved "!" is not effective.
	expect(deviceBot("discord", DISCORD)).toEqual({
		ok: true,
		bot: {
			provider: "discord",
			open: true,
			savedToken: false,
			prefix: "",
			mentions: true,
		},
	});
});

test("absent settings take their defaults; an allow list closes the bot", () => {
	expect(deviceBot("telegram", {})).toEqual({
		ok: true,
		bot: {
			provider: "telegram",
			open: true,
			savedToken: false,
			prefix: "/",
			mentions: true,
		},
	});
	expect(deviceBot("discord", {})).toMatchObject({
		ok: true,
		bot: { prefix: "", mentions: true, open: true },
	});
	expect(
		deviceBot("telegram", {
			chat_whitelist: ["-100123"],
			respond_to_mentions: false,
			command_prefix: "",
		}),
	).toMatchObject({
		ok: true,
		bot: { open: false, mentions: false, prefix: "" },
	});
	// Keys of the other provider, credentials and names are ignored.
	expect(
		deviceBot("discord", {
			chat_whitelist: 7,
			intents: [...DISCORD_INTENTS],
			webhook_secret: 1,
			bot_name: 7,
			sink_type: null,
		}),
	).toMatchObject({ ok: true });
	expect(DISCORD_INTENTS).toHaveLength(19);
	// A key that is `null` takes its default, as on a device.
	expect(
		deviceBot("discord", {
			channel_whitelist: null,
			channel_blacklist: null,
			respond_to_mentions: null,
			respond_to_dms: null,
			command_prefix: null,
			intents: null,
		}),
	).toEqual({
		ok: true,
		bot: {
			provider: "discord",
			open: true,
			savedToken: false,
			prefix: "",
			mentions: true,
		},
	});
});

test("a setting of the wrong JSON type, over its bound or unknown is refused with its key", () => {
	const refused: [string, unknown, string][] = [
		["telegram", null, "config"],
		["telegram", [], "config"],
		["telegram", "{}", "config"],
		["telegram", { chat_whitelist: "-100" }, "chat_whitelist"],
		["telegram", { chat_blacklist: [7] }, "chat_blacklist"],
		["telegram", { chat_whitelist: [""] }, "chat_whitelist"],
		["telegram", { chat_whitelist: ["x".repeat(65)] }, "chat_whitelist"],
		[
			"telegram",
			{ chat_whitelist: Array.from({ length: 257 }, (_, i) => `${i}`) },
			"chat_whitelist",
		],
		["telegram", { respond_to_mentions: "yes" }, "respond_to_mentions"],
		["telegram", { respond_to_private: 1 }, "respond_to_private"],
		["telegram", { command_prefix: 1 }, "command_prefix"],
		["telegram", { command_prefix: "x".repeat(17) }, "command_prefix"],
		["discord", { channel_whitelist: {} }, "channel_whitelist"],
		["discord", { respond_to_dms: 0 }, "respond_to_dms"],
		["discord", { intents: "Guilds" }, "intents"],
		["discord", { intents: ["Guilds", "Everything"] }, "intents: Everything"],
		["discord", { intents: [7] }, "intents: 7"],
		// Of two unreadable settings a device names the same one first.
		["discord", { respond_to_dms: 1, intents: "Guilds" }, "respond_to_dms"],
		["telegram", { chat_blacklist: 1, command_prefix: 5 }, "chat_blacklist"],
	];
	for (const [type, config, key] of refused)
		expect([type, config, deviceBot(type, config)]).toEqual([
			type,
			config,
			{ ok: false, problem: "bot_invalid", detail: key },
		]);
	// Bounds count characters, not bytes: 64 umlauts fit.
	expect(
		deviceBot("telegram", { chat_whitelist: ["ü".repeat(64)] }),
	).toMatchObject({ ok: true });
	expect(
		deviceBot("telegram", { command_prefix: "🤖".repeat(16) }),
	).toMatchObject({ ok: true });
});

test("a Discord bot reads no prefix, so none is effective and none is refused", () => {
	expect(
		deviceBot("discord", { respond_to_mentions: false, command_prefix: "?" }),
	).toMatchObject({ ok: true, bot: { prefix: "", mentions: false } });
	for (const command_prefix of [1, ["!"], "x".repeat(17)])
		expect(deviceBot("discord", { command_prefix })).toMatchObject({
			ok: true,
			bot: { prefix: "" },
		});
});

test("a bot's id must leave room for its token key; any other type is no bot", () => {
	expect(deviceBot("telegram", {}, "e".repeat(112))).toMatchObject({
		ok: true,
	});
	expect(deviceBot("telegram", {}, "e".repeat(113))).toEqual({
		ok: false,
		problem: "bot_invalid",
		detail: "id",
	});
	expect(botTokenKey("e".repeat(112))).toHaveLength(128);
	for (const type of ["teams", "simple_chat", "cron"])
		expect(deviceBot(type, TELEGRAM)).toBeNull();
});

test("the token key and the handle are the reserved spellings", () => {
	expect(botTokenKey("evt_helper")).toBe("event.evt_helper.bot_token");
	expect(isBotTokenKey("event.evt_helper.bot_token")).toBe(true);
	expect(botTokenEventId("event.evt.a-1.bot_token")).toBe("evt.a-1");
	for (const key of [
		"evt_helper.bot_token",
		"event..bot_token",
		"event.evt_helper.token",
		"variable-1",
		`event.${"e".repeat(113)}.bot_token`,
	]) {
		expect(isBotTokenKey(key)).toBe(false);
		expect(botTokenEventId(key)).toBeNull();
	}
	expect(botHandle("evt_helper")).toBe("device-bot:evt_helper");
	expect(botTokenVariable({ id: "evt_helper", name: "Helper" })).toEqual({
		id: "event.evt_helper.bot_token",
		name: "Helper",
		data_type: "String",
		value_type: "Normal",
		secret: true,
	});
});

test("token shapes catch a pasted sentence or a cut token, without network", () => {
	const discord = `${"M".repeat(24)}.${"G".repeat(6)}.${"x".repeat(38)}`;
	expect(
		botTokenProblem("telegram", "123456789:AAH-secret_token_value_123"),
	).toBeNull();
	expect(
		botTokenProblem("telegram", " 123:abcdefghijklmnopqrst \n"),
	).toBeNull();
	expect(botTokenProblem("discord", discord)).toBeNull();
	for (const [provider, token] of [
		["telegram", "12:abcdefghijklmnopqrstu"],
		["telegram", "123456789:short"],
		["telegram", "123456789 AAH-secret_token_value_123"],
		["telegram", `123:${"a".repeat(129)}`],
		["telegram", "my token is 123456789:AAH-secret_token_value_123"],
		["discord", "a.b.c"],
		["discord", `${"M".repeat(24)}.${"G".repeat(6)}`],
		["discord", `${"M".repeat(120)}.${"G".repeat(60)}.${"x".repeat(80)}`],
		["discord", discord.replace("x", "+")],
	] as const)
		expect([token, botTokenProblem(provider, token)]).toEqual([token, "shape"]);
	expect(botTokenProblem("telegram", "  ")).toBe("empty");
	expect(botTokenProblem("discord", "")).toBe("empty");
});

test("the token saved in Events is offered by provider key, never another", () => {
	expect(savedBotToken("telegram", TELEGRAM)).toBe(
		"123456789:AAH-secret_token_value_123",
	);
	expect(savedBotToken("discord", { ...DISCORD, token: " abc " })).toBe("abc");
	expect(savedBotToken("discord", DISCORD)).toBeNull();
	expect(savedBotToken("discord", { bot_token: "x" })).toBeNull();
	expect(savedBotToken("telegram", { token: "x" })).toBeNull();
	expect(savedBotToken("teams", TELEGRAM)).toBeNull();
	expect(savedBotToken("telegram", null)).toBeNull();
});
