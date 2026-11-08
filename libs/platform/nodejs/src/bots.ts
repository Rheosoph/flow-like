import type { HttpClient } from "./client.js";
import { segment } from "./paths.js";
import type { JsonObject } from "./types.js";

/** Something a bot may do. Every capability is off until the owner switches it on. */
export type BotCapability =
	| "internet"
	| "profile_apps"
	| "selected_apps"
	| "code_execution"
	| "computer_use"
	| "browser_use"
	| "email"
	| "push_notifications"
	| "scheduling"
	| "storage"
	| "data_studio"
	| "command_line"
	| "encryption"
	| "user_context"
	| "webhook";

/** Where the bot's host runs. A `server` bot can't hold `computer_use`, `browser_use` or `command_line`. */
export type BotLocation = "server" | "desktop" | "device";

/** The built-in bots every person has: `general` (Assistant) and `flux`. */
export type BotSystemKind = "general" | "flux";

export type DataClassification =
	| "public"
	| "internal"
	| "confidential"
	| "restricted";

export interface BotModelPolicy {
	/** Bit id of the preferred model; null lets the host choose. */
	primary: string | null;
	/** Bit ids hosts try in order where the primary is unavailable; at most 3. */
	fallbacks: string[];
	/** System One Bit for response decisions; absent or null disables it. */
	decision?: string | null;
}

export interface BotClassification {
	/** Absent when nobody classified the bot's data. */
	classification?: DataClassification;
	pii: boolean;
}

export interface Bot {
	/** Also the bot's sub: records of what the bot did store it. */
	id: string;
	owner_id: string;
	/** Null for bots you created. */
	system: BotSystemKind | null;
	preset_id: string | null;
	/** Null on a built-in bot that was never renamed: show its localized default name. */
	name: string | null;
	description: string | null;
	instructions: string | null;
	/** The look from the bot builder: an object with a string `kind`. */
	appearance: JsonObject;
	model: BotModelPolicy;
	classification: BotClassification;
	capabilities: BotCapability[];
	/** The profile whose apps `profile_apps` covers. */
	profile_id: string | null;
	location: BotLocation;
	/** The managed device a `device` bot runs on. */
	device_id: string | null;
	/** The local id a bot moved from a device was created with. */
	import_key?: string | null;
	created_at: string;
	updated_at: string;
}

export interface CreateBotRequest {
	/** 1 to 64 characters. */
	name: string;
	/** The look from the bot builder; defaults to `{ kind: "pip" }`, the look Assistant starts with. */
	appearance?: JsonObject;
	preset_id?: string;
	description?: string;
	instructions?: string;
	model?: Partial<BotModelPolicy>;
	classification?: Partial<BotClassification>;
	/** The bot starts with these and nothing else; `profile_apps` needs `profile_id`. */
	capabilities?: BotCapability[];
	profile_id?: string;
	/** `server` by default. */
	location?: BotLocation;
	/** Required for `device` bots: one of your active managed devices. */
	device_id?: string;
}

/** Omitted fields stay as they are; `null` clears a nullable field. */
export interface UpdateBotRequest {
	/** `null` resets a built-in bot to its localized default name. */
	name?: string | null;
	description?: string | null;
	instructions?: string | null;
	appearance?: JsonObject;
	/** Replaces the whole policy. */
	model?: Partial<BotModelPolicy>;
	classification?: Partial<BotClassification>;
	/** Replaces the whole set. Flux's capabilities can't be changed. */
	capabilities?: BotCapability[];
	profile_id?: string | null;
	location?: BotLocation;
	device_id?: string | null;
}

export interface BotApp {
	app_id: string;
	added_at: string;
	/** Your current role in the app, which bounds the bot there; null once you left the app. */
	owner_role: string | null;
}

export type BotSecretEncryption = "server" | "sealed";

/** A secret's metadata. No response ever carries its value. */
export interface BotSecret {
	name: string;
	encryption: BotSecretEncryption;
	description: string | null;
	created_at: string;
	updated_at: string;
	last_used_at: string | null;
}

/** Exactly one of `value` (encrypted by the server) or `sealed` (an envelope sealed to the bot's key). */
export type SetBotSecretRequest = (
	| { value: string; sealed?: never }
	| { sealed: JsonObject; value?: never }
) & {
	/** Omitted keeps the current description; `null` clears it. */
	description?: string | null;
};

const DEFAULT_APPEARANCE: JsonObject = { kind: "pip" };

const botPath = (botId: string) => `/bots/${segment(botId)}`;

/** Bots act for their owner. These routes accept people only (PATs), not API keys. */
export function createBotMethods(http: HttpClient) {
	return {
		/** Assistant and Flux first (set up on the first call), then your bots, oldest first. */
		async listBots(): Promise<Bot[]> {
			return (await http.request<{ bots: Bot[] }>("GET", "/bots")).bots;
		},
		getBot(botId: string): Promise<Bot> {
			return http.request("GET", botPath(botId));
		},
		createBot(request: CreateBotRequest): Promise<Bot> {
			return http.request("POST", "/bots", {
				body: {
					...request,
					appearance: request.appearance ?? DEFAULT_APPEARANCE,
				},
			});
		},
		updateBot(botId: string, changes: UpdateBotRequest): Promise<Bot> {
			return http.request("PATCH", botPath(botId), { body: changes });
		},
		/** Assistant and Flux can't be deleted (409 `BOT_SYSTEM_UNDELETABLE`). */
		async deleteBot(botId: string): Promise<void> {
			await http.request("DELETE", botPath(botId));
		},
		async listBotApps(botId: string): Promise<BotApp[]> {
			return (
				await http.request<{ apps: BotApp[] }>("GET", `${botPath(botId)}/apps`)
			).apps;
		},
		/** Lets the bot use an app you are a member of, with your role there. */
		async attachBotApp(botId: string, appId: string): Promise<void> {
			await http.request("PUT", `${botPath(botId)}/apps/${segment(appId)}`);
		},
		async detachBotApp(botId: string, appId: string): Promise<void> {
			await http.request("DELETE", `${botPath(botId)}/apps/${segment(appId)}`);
		},
		/** Names and metadata only; values are write-only. */
		async listBotSecrets(botId: string): Promise<BotSecret[]> {
			return (
				await http.request<{ secrets: BotSecret[] }>(
					"GET",
					`${botPath(botId)}/secrets`,
				)
			).secrets;
		},
		/** Stores or replaces a secret named like an environment variable (`^[A-Z][A-Z0-9_]{0,63}$`). */
		setBotSecret(
			botId: string,
			name: string,
			secret: SetBotSecretRequest,
		): Promise<BotSecret> {
			return http.request("PUT", `${botPath(botId)}/secrets/${segment(name)}`, {
				body: secret,
			});
		},
		async deleteBotSecret(botId: string, name: string): Promise<void> {
			await http.request(
				"DELETE",
				`${botPath(botId)}/secrets/${segment(name)}`,
			);
		},
	};
}
