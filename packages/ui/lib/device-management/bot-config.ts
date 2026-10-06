/*
 * The bot settings rule a device applies to a `telegram` or `discord` event
 * (design R2 §1.4), the desktop app's reading of the same settings without a
 * device's bounds, and the bot token's place in a placement config (§1.10).
 * A device never receives the token in the event's config: it is a secret of
 * the service, referenced by a reserved key.
 */

export type BotProvider = "telegram" | "discord";

export const BOT_EVENT_TYPES = ["telegram", "discord"] as const;

/** What the What step and the Events editor say about a bot; `prefix` and `mentions` are the effective values. */
export interface BotFacts {
	provider: BotProvider;
	/** No allowed chats or channels: anyone who can message the bot starts runs. */
	open: boolean;
	/** The event record carries a token of its own (offered in Settings, never shown). */
	savedToken: boolean;
	/** Empty when no command prefix is set. Never trimmed: a blank is a prefix. */
	prefix: string;
	mentions: boolean;
}

/** Which messages of a group or server channel start a run. */
export type BotAnswerCase =
	| "prefix_and_mentions"
	| "prefix_only"
	| "mentions"
	| "every";

export type DeviceBotResult =
	| { ok: true; bot: BotFacts }
	| {
			ok: false;
			problem: "bot_invalid";
			/** The setting a device can't read: its key, and an unknown intent's name. */
			detail: string;
	  };

/** The intents a device knows (`desk/event_sink/discord.rs`). */
export const DISCORD_INTENTS = [
	"Guilds",
	"GuildMembers",
	"GuildModeration",
	"GuildEmojisAndStickers",
	"GuildIntegrations",
	"GuildWebhooks",
	"GuildInvites",
	"GuildVoiceStates",
	"GuildPresences",
	"GuildMessages",
	"GuildMessageReactions",
	"GuildMessageTyping",
	"DirectMessages",
	"DirectMessageReactions",
	"DirectMessageTyping",
	"MessageContent",
	"GuildScheduledEvents",
	"AutoModerationConfiguration",
	"AutoModerationExecution",
] as const;

const SETTINGS: Record<
	BotProvider,
	{ allow: string; deny: string; private: string }
> = {
	telegram: {
		allow: "chat_whitelist",
		deny: "chat_blacklist",
		private: "respond_to_private",
	},
	discord: {
		allow: "channel_whitelist",
		deny: "channel_blacklist",
		private: "respond_to_dms",
	},
};

const MAX_LIST = 256;
const MAX_ENTRY = 64;
const MAX_PREFIX = 16;
/** `event.<id>.bot_token` is 16 characters longer than the id and a key has at most 128. */
export const MAX_BOT_EVENT_ID = 112;

const characters = (text: string) => [...text].length;

export function botProvider(eventType: string): BotProvider | null {
	return eventType === "telegram" || eventType === "discord" ? eventType : null;
}

type Check = (value: unknown) => boolean;

/** How a reader takes the settings a device bounds. */
interface Reader {
	list: Check;
	prefix: Check;
	/** The key of the first intent it can't take; null when it takes them all. */
	intents: (intents: unknown) => string | null;
}

const text = (value: unknown) => typeof value === "string";

const texts = (value: unknown) => Array.isArray(value) && value.every(text);

const flag = (value: unknown) => typeof value === "boolean";

const readEntry = (entry: unknown) =>
	typeof entry === "string" &&
	characters(entry) >= 1 &&
	characters(entry) <= MAX_ENTRY;

const knownIntent = (intent: unknown) =>
	typeof intent === "string" &&
	(DISCORD_INTENTS as readonly string[]).includes(intent);

/** The key of the first unknown intent; null when every intent is known. */
function intentProblem(intents: unknown): string | null {
	if (intents == null) return null;
	if (!Array.isArray(intents)) return "intents";
	const unknown = intents.find((intent) => !knownIntent(intent));
	return unknown === undefined
		? null
		: `intents: ${String(unknown).slice(0, 64)}`;
}

const DEVICE: Reader = {
	list: (value) =>
		Array.isArray(value) && value.length <= MAX_LIST && value.every(readEntry),
	prefix: (value) =>
		typeof value === "string" && characters(value) <= MAX_PREFIX,
	intents: intentProblem,
};

/** `BotSpec::unbounded`: the same JSON types without a bound; an intent it doesn't know is skipped. */
const DESKTOP: Reader = {
	list: texts,
	prefix: text,
	intents: (intents) => (intents == null || texts(intents) ? null : "intents"),
};

const LONE_SURROGATE = /\p{Cs}/u;

/** serde_json, which a device and the desktop app parse a config with, refuses a lone surrogate in any key or text. */
function wellFormed(config: object): boolean {
	const pending: unknown[] = [config];
	while (pending.length > 0) {
		const value = pending.pop();
		if (typeof value === "string" && LONE_SURROGATE.test(value)) return false;
		if (value && typeof value === "object")
			for (const [key, entry] of Object.entries(value))
				pending.push(key, entry);
	}
	return true;
}

const jsonObject = (config: unknown): config is Record<string, unknown> =>
	typeof config === "object" &&
	config !== null &&
	!Array.isArray(config) &&
	wellFormed(config);

/** The first setting the reader can't take, in the order a device checks them; a key that is absent or `null` takes its default. */
function settingProblem(
	provider: BotProvider,
	config: Record<string, unknown>,
	read: Reader,
): string | null {
	const keys = SETTINGS[provider];
	const rules: [string, Check][] = [
		[keys.allow, read.list],
		[keys.deny, read.list],
		["respond_to_mentions", flag],
		[keys.private, flag],
		["command_prefix", read.prefix],
	];
	const failed = rules.find(
		([key, valid]) => config[key] != null && !valid(config[key]),
	);
	if (failed) return failed[0];
	return provider === "discord" ? read.intents(config.intents) : null;
}

const refused = (detail: string): DeviceBotResult => ({
	ok: false,
	problem: "bot_invalid",
	detail,
});

function readBot(
	provider: BotProvider,
	config: unknown,
	read: Reader,
): DeviceBotResult {
	if (!jsonObject(config)) return refused("config");
	const problem = settingProblem(provider, config, read);
	if (problem) return refused(problem);
	const allow = config[SETTINGS[provider].allow];
	const prefix = config.command_prefix;
	return {
		ok: true,
		bot: {
			provider,
			open: !Array.isArray(allow) || allow.length === 0,
			savedToken: savedBotToken(provider, config) !== null,
			prefix: typeof prefix === "string" ? prefix : "",
			mentions: config.respond_to_mentions !== false,
		},
	};
}

/**
 * The bot a device runs for this event, or why it can't read its settings.
 * Null for an event that is no bot. The event's own token and webhook secret
 * are never read here, only whether a token is saved.
 */
export function deviceBot(
	eventType: string,
	config: unknown,
	eventId?: string,
): DeviceBotResult | null {
	const provider = botProvider(eventType);
	if (!provider) return null;
	if (eventId !== undefined && characters(eventId) > MAX_BOT_EVENT_ID)
		return refused("id");
	return readBot(provider, config, DEVICE);
}

/**
 * The bot the desktop app runs for this event: the settings a device reads,
 * without a device's bounds. Null for an event that is no bot or a setting of
 * a JSON type the desktop app can't read; the token is not read here either.
 */
export function desktopBot(
	eventType: string,
	config: unknown,
): BotFacts | null {
	const provider = botProvider(eventType);
	if (!provider) return null;
	const result = readBot(provider, config, DESKTOP);
	return result.ok ? result.bot : null;
}

/**
 * What a bot answers in a group or server channel its lists allow: the client's
 * copy of the rule a device and the desktop app decide by (`BotSpec::admits`).
 * With a prefix it answers messages that start with it, plus mentions and
 * replies while `mentions` is on. Without one it answers every message, except
 * a Discord bot with `mentions` on, which answers mentions and replies only.
 */
export function botAnswerCase(
	bot: Pick<BotFacts, "provider" | "prefix" | "mentions">,
): BotAnswerCase {
	if (bot.prefix !== "")
		return bot.mentions ? "prefix_and_mentions" : "prefix_only";
	return bot.provider === "discord" && bot.mentions ? "mentions" : "every";
}

const TOKEN_KEY = /^event\.([A-Za-z0-9_.-]{1,112})\.bot_token$/;

/** The reserved `secret_overrides` key of a bot event's token. */
export function botTokenKey(eventId: string): string {
	return `event.${eventId}.bot_token`;
}

export function isBotTokenKey(key: string): boolean {
	return TOKEN_KEY.test(key);
}

/** The event a bot token key belongs to; null for any other key. */
export function botTokenEventId(key: string): string | null {
	return TOKEN_KEY.exec(key)?.[1] ?? null;
}

/** What a flow receives in place of the token. */
export function botHandle(eventId: string): string {
	return `device-bot:${eventId}`;
}

const TELEGRAM_TOKEN = /^[0-9]{3,20}:[A-Za-z0-9_-]{20,128}$/;
const DISCORD_TOKEN = /^[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+$/;

/**
 * A shape check without network: it catches a pasted sentence or a cut token.
 * Surrounding white space is ignored; send the trimmed token.
 */
export function botTokenProblem(
	provider: BotProvider,
	token: string,
): "empty" | "shape" | null {
	const value = token.trim();
	if (!value) return "empty";
	const fits =
		provider === "telegram"
			? TELEGRAM_TOKEN.test(value)
			: DISCORD_TOKEN.test(value) && value.length >= 40 && value.length <= 256;
	return fits ? null : "shape";
}

/**
 * The token saved on the event record, for the Settings step only. Never
 * logged, never kept in a draft: it travels only inside `set_secret` or
 * `rollout_secret`.
 */
export function savedBotToken(
	eventType: string,
	config: unknown,
): string | null {
	const provider = botProvider(eventType);
	if (!provider || !config || typeof config !== "object") return null;
	const record = config as Record<string, unknown>;
	const token = provider === "telegram" ? record.bot_token : record.token;
	return typeof token === "string" && token.trim() ? token.trim() : null;
}

/**
 * The synthetic secret setting of a bot event's token, so Settings, "Keep
 * stored / Set new", per-device values and secret publication treat it like
 * any secret variable.
 */
export function botTokenVariable(event: { id: string; name: string }) {
	return {
		id: botTokenKey(event.id),
		name: event.name.slice(0, 120),
		data_type: "String",
		value_type: "Normal",
		secret: true,
	};
}
