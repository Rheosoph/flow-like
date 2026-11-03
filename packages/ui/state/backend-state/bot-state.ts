import type { BotAppearance } from "../../components/bots/types";
import type { IDataSensitivity } from "../../lib/data-classification";

/** Wire ids, in the order the settings list them. New bots start with `memory`; the rest stays off until switched on. */
export const BOT_CAPABILITIES = [
	"internet",
	"profile_apps",
	"selected_apps",
	"code_execution",
	"computer_use",
	"browser_use",
	"email",
	"push_notifications",
	"scheduling",
	"storage",
	"data_studio",
	"memory",
	"documents",
	"data_connections",
	"knowledge",
	"connectors",
	"channels",
	"skills",
	"image_generation",
	"delegation",
	"command_line",
	"encryption",
	"webhook",
	"flowpilot",
] as const;
export type BotCapability = (typeof BOT_CAPABILITIES)[number];

/** Flux's complete toolset (FlowScript, building, debugging). Fixed: only Flux and its duplicates have it. */
export const FLOWPILOT_CAPABILITY: BotCapability = "flowpilot";

/** Recalls and saves what matters from the bot's chats. FlowPilot bots always remember. */
export const MEMORY_CAPABILITY: BotCapability = "memory";

export const BOT_LOCATIONS = ["server", "desktop", "device"] as const;
export type BotLocation = (typeof BOT_LOCATIONS)[number];

/** Need a screen, a shell or a key only the host holds, so a bot on the server can't hold them. */
export const LOCAL_ONLY_BOT_CAPABILITIES: readonly BotCapability[] = [
	"computer_use",
	"browser_use",
	"command_line",
	"encryption",
];

export function isBotCapabilityAvailable(
	capability: BotCapability,
	location: BotLocation,
): boolean {
	return (
		location !== "server" || !LOCAL_ONLY_BOT_CAPABILITIES.includes(capability)
	);
}

export type BotSystemKey = "general" | "flux";

/** The signed-out person on a desktop: owner of its local bots and sender of its local messages. */
export const LOCAL_OWNER_ID = "local";

/** The look each built-in bot starts with; the server gives new built-in bots the same. */
export const SYSTEM_BOT_APPEARANCES: Readonly<
	Record<BotSystemKey, BotAppearance>
> = {
	general: { kind: "pip" },
	flux: { kind: "flux" },
};

/** What a built-in bot can do while it has no row of its own; the hosts give new built-in rows the same. */
export const SYSTEM_BOT_CAPABILITIES: Readonly<
	Record<BotSystemKey, readonly BotCapability[]>
> = {
	general: ["internet", MEMORY_CAPABILITY],
	flux: [FLOWPILOT_CAPABILITY],
};

/** Bit ids. Hosts that can't run the primary try the fallbacks in order. */
export interface IBotModelPolicy {
	primary: string | null;
	fallbacks: string[];
}

export interface IBot {
	/** Also the bot's sub: records of what the bot did store this id. */
	id: string;
	owner_id: string;
	system: BotSystemKey | null;
	preset_id: string | null;
	/** Null on a built-in bot the owner never renamed; show the localized default. */
	name: string | null;
	description: string | null;
	instructions: string | null;
	appearance: BotAppearance;
	model: IBotModelPolicy;
	/** Bit id of the embedding model its memory uses; unset lets the host choose. */
	embedding_model?: string | null;
	classification: IDataSensitivity;
	capabilities: BotCapability[];
	/** The profile whose apps `profile_apps` covers. */
	profile_id: string | null;
	location: BotLocation;
	/** Set when `location` is `device`. */
	device_id: string | null;
	created_at: string;
	updated_at: string;
	/** Lives only on this device until it moves to an account. */
	local?: boolean;
}

export interface ICreateBotRequest {
	name: string;
	preset_id?: string | null;
	description?: string | null;
	instructions?: string | null;
	appearance: BotAppearance;
	model?: IBotModelPolicy;
	embedding_model?: string | null;
	classification?: IDataSensitivity;
	capabilities?: BotCapability[];
	profile_id?: string | null;
	location?: BotLocation;
	device_id?: string | null;
}

/** Omitted fields stay as they are; `null` clears a nullable field. */
export interface IUpdateBotRequest {
	name?: string | null;
	description?: string | null;
	instructions?: string | null;
	appearance?: BotAppearance;
	model?: IBotModelPolicy;
	embedding_model?: string | null;
	classification?: IDataSensitivity;
	capabilities?: BotCapability[];
	profile_id?: string | null;
	location?: BotLocation;
	device_id?: string | null;
}

export interface IBotApp {
	app_id: string;
	added_at: string;
	/** The owner's current role in the app; null once the owner is no longer a member. */
	owner_role: string | null;
}

/** `device`: a local bot's secret, encrypted with a key kept in this device's keychain. */
export type BotSecretEncryption = "server" | "sealed" | "device";

/** Secrets are write-only: no response ever carries a value. */
export interface IBotSecretInfo {
	name: string;
	encryption: BotSecretEncryption;
	description: string | null;
	created_at: string;
	updated_at: string;
	last_used_at: string | null;
}

/** Exactly one of `value` (encrypted by the server) or `sealed` (an envelope sealed to the bot's key). */
export interface IUpsertBotSecretRequest {
	value?: string;
	sealed?: Record<string, unknown>;
	description?: string | null;
}

export const BOT_SECRET_NAME_PATTERN = /^[A-Z][A-Z0-9_]{0,63}$/;

export const BOT_LIMITS = {
	name: 64,
	description: 2000,
	instructions: 8000,
	fallbacks: 3,
	secrets: 50,
	secretValueBytes: 16 * 1024,
} as const;

/** `bot`, `profile`, `device`, `app` and `capability` come from the server's check; the rest from the client. */
export type BotMovePart =
	| "bot"
	| "profile"
	| "device"
	| "app"
	| "capability"
	| "model"
	| "look"
	| "secret";

export interface IBotMoveIssue {
	part: BotMovePart;
	/** The server's error code (`BOT_APP_UNAVAILABLE`, …) or a client check's (`BOT_MODEL_UNAVAILABLE`). */
	code: string;
	message: string;
	/** The app, profile, device, capability or model the issue is about. */
	ref_id: string | null;
}

/** What moving one local bot to the signed-in account takes along, and what blocks it. */
export interface IBotMoveReport {
	bot_id: string;
	/** Empty when nothing blocks the move. */
	issues: IBotMoveIssue[];
	apps: number;
	secrets: number;
	/** Chats that move with the bot. */
	conversations: number;
	messages: number;
	/** Chats that also hold local bots staying behind; they move once those bots do. */
	waiting_conversations: number;
	/** Messages from bots that left these chats and aren't in the account; they don't move. */
	skipped_messages: number;
}

/** The steps of a move, in the order the desktop tells them; `workspace`, `segments` and `forget_workspace` only for bots that have them. */
export const BOT_MOVE_STEPS = [
	"check",
	"import_bot",
	"update_bot",
	"apps",
	"secrets",
	"workspace",
	"conversations",
	"segments",
	"forget_conversations",
	"verify",
	"forget_workspace",
	"rebind",
	"forget_bot",
	"done",
] as const;
export type BotMoveStep = (typeof BOT_MOVE_STEPS)[number];

/** Told before each step of a move (`local-bots://move`). */
export interface IBotMoveProgress {
	bot_id: string;
	step: BotMoveStep;
	/** The export, import, forget pass, from 1; 0 before the first. */
	pass: number;
}

/** A finished move. */
export interface IMovedBot {
	/** The account's bot. */
	bot: IBot;
	/** File links and summaries that couldn't move and were left out. */
	warnings: number;
}

/** Desktop only: moves bots that live on this device to the signed-in account. */
export interface ILocalBotHost {
	/** Every bot on this computer, built-ins a signed-in list leaves out (the account has its own) included. */
	list(): Promise<IBot[]>;
	/** Checks every part of a local bot against the signed-in account without changing anything. */
	checkMove(botId: string): Promise<IBotMoveReport>;
	/**
	 * Moves the bot with its apps, secrets and chats, then removes the local copy. The account
	 * mints the bot's new id (returned) and its chats' ids; Assistant and Flux edits go onto the
	 * account's own built-ins. Refused while the check reports issues; repeating it after an
	 * interruption is safe.
	 */
	move(botId: string): Promise<IMovedBot>;
	/** Listens to the steps of moves; resolves to the function that stops listening. */
	onProgress?(
		listener: (progress: IBotMoveProgress) => void,
	): Promise<() => void>;
}

export type BotPresenceState = "online" | "offline" | "unknown";

/** Whether a host answers for an account bot right now. */
export interface IBotPresence {
	state: BotPresenceState;
	/** When a host last reported the bot; absent for server bots and bots never seen. */
	last_seen_at?: string | null;
}

/** One bot allowed to ask another without asking the person again. */
export interface IBotPermission {
	id: string;
	/** The bot that may ask. */
	from_bot_id: string;
	/** The bot that may be asked. */
	to_bot_id: string;
	granted_at: string;
	granted_by: string;
}

export interface IBotPermissions {
	/** Bots this bot may ask. */
	granted: IBotPermission[];
	/** Bots that may ask this bot. */
	received: IBotPermission[];
}

/** What the owner may answer to an approval card. */
export type BotApprovalDecision = "once" | "always" | "deny";

/** A call the owner allowed for good: `tool` for `scope` (`*` covers every scope of the tool). */
export interface IBotToolGrant {
	tool: string;
	scope: string;
	granted_at: string;
	granted_by: string;
}

export type BotTurnStatus =
	| "queued"
	| "running"
	| "waiting"
	| "done"
	| "failed"
	| "cancelled";

/** A turn a bot works on without a chat window (`bot-host://turn`). */
export interface IBotTurn {
	/** Also the run id. */
	id: string;
	bot_id: string;
	conversation_id: string;
	status: BotTurnStatus;
	started_at?: string | null;
	finished_at?: string | null;
	error_code?: string | null;
	/** The approval card a `waiting` turn sleeps on. */
	reply_message_id?: string | null;
}

/** Desktop only: the turns the bot host works on, for bots on this computer and account bots that run here. */
export interface IBotTurnHost {
	list(): Promise<IBotTurn[]>;
	/** Called whenever a turn changes state; resolves to the function that stops listening. */
	subscribe(listener: (turn: IBotTurn) => void): Promise<() => void>;
	/** Ends the turn without a reply. */
	cancel(turn: IBotTurn): Promise<void>;
}

/** The inbox key an encryption bot holds on the host that runs it. */
export interface IBotEncryptionStatus {
	enabled: boolean;
	key_id?: string | null;
	fingerprint?: string | null;
	/** Whether the public key is in the key directory. */
	published: boolean;
	/** The host that holds the private key. */
	host_id: string;
}

/** Desktop only: end-to-end encryption keys of account bots that run here. */
export interface IBotEncryptionHost {
	status(botId: string): Promise<IBotEncryptionStatus>;
	/** Publishes a new key and revokes the old one; earlier messages still open. */
	rotate(botId: string): Promise<IBotEncryptionStatus>;
}

/** What the bot panel's Computer tab shows: the latest picture of the bot's browser or screen. */
export interface IBotComputerView {
	state: "asleep" | "active";
	kind: "browser" | "computer" | null;
	/** The page address for a browser. */
	caption: string | null;
	/** A `data:` URL of the picture. */
	image: string | null;
	age_ms: number | null;
}

/** Desktop only: the browser and screen of the bots that run here. */
export interface IBotComputerHost {
	view(botId: string): Promise<IBotComputerView>;
	/** Closes the bot's browser and stops the mouse and keyboard action in flight. */
	stop(botId: string): Promise<void>;
}

/** A bot's own hidden app: its files and databases (and, with the memory phase, its memory). */
export interface IBotWorkspace {
	app_id: string;
}

/** A public URL that wakes a bot; the secret is never part of it. */
export interface IBotWebhook {
	id: string;
	bot_id: string;
	name: string;
	/** What the bot should do with each call. */
	instruction: string;
	enabled: boolean;
	/** Call it with POST under the API base. */
	path: string;
	created_at: string;
	last_called_at?: string | null;
}

/** A new or rotated webhook with the only copy of its credentials. */
export interface IIssuedBotWebhook {
	webhook: IBotWebhook;
	secret: string;
	/** Key for `X-Flow-Like-Signature: sha256=<hmac of the body>`. */
	signing_key: string;
}

/** A bot's own mail address on the platform's mail domain. */
export interface IBotMailAddress {
	/** Whether this server receives email at all. */
	configured: boolean;
	domain?: string | null;
	address?: string | null;
	alias?: string | null;
	/** The bot has the email ability on. */
	enabled: boolean;
}

export interface IBotMailAvailability {
	available: boolean;
	reason?: string | null;
}

/** One mail thread of a bot: a conversation of the bot with its owner, marked as mail. */
export interface IBotMailThread {
	id: string;
	subject: string;
	participants: string[];
	last_message_at: string;
	last_snippet?: string | null;
	message_count: number;
	unread: number;
	created_at: string;
}

export interface IMailAddress {
	name?: string | null;
	email?: string | null;
}

export interface IBotMailAttachment {
	name: string;
	content_type: string;
	size: number;
	/** The file in the bot's workspace; open it in Files. */
	handle?: {
		app_id: string;
		path: string;
		bytes: number;
		sha256: string;
		mime: string;
	};
	skipped?: string;
}

export interface IBotMail {
	id: string;
	seq?: number | null;
	direction: "in" | "out";
	at: string;
	from: IMailAddress[] | string;
	to: IMailAddress[] | string[];
	cc: IMailAddress[] | string[];
	subject: string;
	text: string;
	/** False when the sender could not be verified. */
	authenticated: boolean;
	attachments: IBotMailAttachment[];
}

export interface IBotMailThreadView {
	thread: IBotMailThread;
	mails: IBotMail[];
}

/** A task a bot runs for its owner at a time: once or on a cron schedule in a time zone. */
export interface IBotTask {
	id: string;
	bot_id: string;
	kind: "once" | "cron";
	/** RFC 3339 time of a once task. */
	at?: string | null;
	/** Five-field cron expression of a cron task. */
	cron?: string | null;
	/** IANA time zone of the cron expression. */
	tz?: string | null;
	instruction: string;
	title: string;
	next_run_at?: string | null;
	last_run_at?: string | null;
	enabled: boolean;
	created_by: "bot" | "owner";
	created_at: string;
}

export interface ICreateBotTaskRequest {
	kind: "once" | "cron";
	at?: string;
	cron?: string;
	tz?: string;
	instruction: string;
	title?: string;
}

export interface IUpdateBotTaskRequest {
	at?: string;
	cron?: string;
	tz?: string;
	instruction?: string;
	title?: string;
	/** Pausing keeps the task; resuming computes its next run from now. */
	enabled?: boolean;
}

export interface IBotState {
	/** Present where bots can also live on this device (desktop). Signed out, every bot is local. */
	readonly local?: ILocalBotHost;
	/** Present on the desktop, which runs bots without a chat window. */
	readonly turns?: IBotTurnHost;
	/** Present on the desktop, which holds the keys of its encryption bots. */
	readonly encryption?: IBotEncryptionHost;
	/** Present on the desktop, which can show what its bots do in a browser or on the screen. */
	readonly computer?: IBotComputerHost;
	/** Per bot id, for the account's bots. Absent where the hub has no presence. */
	getPresence?(): Promise<Record<string, IBotPresence>>;
	/** Standing grants of one bot: whom it may ask and who may ask it. */
	listPermissions(botId: string): Promise<IBotPermissions>;
	/** Lets `fromBotId` ask `toBotId` without asking the person; repeating returns the first grant. Both bots live in the same place. */
	grantPermission(fromBotId: string, toBotId: string): Promise<IBotPermission>;
	/** Takes the grant back; none is not an error. */
	revokePermission(fromBotId: string, toBotId: string): Promise<void>;
	/** Standing tool approvals of a bot ("Always allow" / "Don't ask again"), kept with the bot. */
	listToolGrants?(botId: string): Promise<IBotToolGrant[]>;
	/** Repeating returns the first grant. */
	putToolGrant?(
		botId: string,
		tool: string,
		scope: string,
	): Promise<IBotToolGrant>;
	/** Takes the grant back; none is not an error. */
	deleteToolGrant?(botId: string, tool: string, scope: string): Promise<void>;
	/** Answers the approval card a parked turn sleeps on; the turn runs again unless it was denied. */
	answerApproval?(
		botId: string,
		turnId: string,
		approvalId: string,
		decision: BotApprovalDecision,
	): Promise<void>;
	/** Scheduled tasks of a bot with the scheduling capability, soonest first. */
	listTasks?(botId: string): Promise<IBotTask[]>;
	createTask?(botId: string, request: ICreateBotTaskRequest): Promise<IBotTask>;
	updateTask?(
		botId: string,
		taskId: string,
		request: IUpdateBotTaskRequest,
	): Promise<IBotTask>;
	deleteTask?(botId: string, taskId: string): Promise<void>;
	/** The bot's own mail address; absent where bots have no mail domain. */
	getMailAddress?(botId: string): Promise<IBotMailAddress>;
	checkMailAddress?(
		botId: string,
		alias: string,
	): Promise<IBotMailAvailability>;
	claimMailAddress?(botId: string, alias: string): Promise<IBotMailAddress>;
	releaseMailAddress?(botId: string): Promise<void>;
	listMailThreads?(botId: string): Promise<IBotMailThread[]>;
	getMailThread?(botId: string, threadId: string): Promise<IBotMailThreadView>;
	/** Webhooks of an account bot; absent where bots have no public URL. */
	listWebhooks?(botId: string): Promise<IBotWebhook[]>;
	createWebhook?(
		botId: string,
		request: { name: string; instruction?: string },
	): Promise<IIssuedBotWebhook>;
	/** The old secret stops working at once. */
	rotateWebhook?(botId: string, hookId: string): Promise<IIssuedBotWebhook>;
	deleteWebhook?(botId: string, hookId: string): Promise<void>;
	/** The full URL of a webhook on the hub its bot lives on. */
	webhookUrl?(botId: string, webhook: IBotWebhook): Promise<string>;
	/** Creates the workspace on first use; repeating it returns the same app. Refused for Flux for now. */
	getWorkspace?(botId: string): Promise<IBotWorkspace>;
	/**
	 * A new custom bot with the same name (marked as a copy), look, texts, model, classification,
	 * abilities, location and attached apps. Never secrets, workspace, memory or history.
	 */
	cloneBot?(botId: string): Promise<IBot>;
	/** The caller's bots, built-in bots first. Creates missing built-in bots on the way. */
	listBots(): Promise<IBot[]>;
	getBot(botId: string): Promise<IBot>;
	createBot(request: ICreateBotRequest): Promise<IBot>;
	updateBot(botId: string, request: IUpdateBotRequest): Promise<IBot>;
	/** Refused for built-in bots. */
	deleteBot(botId: string): Promise<void>;
	listBotApps(botId: string): Promise<IBotApp[]>;
	/** The owner must be a member of the app; the app itself is not asked. */
	attachBotApp(botId: string, appId: string): Promise<void>;
	detachBotApp(botId: string, appId: string): Promise<void>;
	listBotSecrets(botId: string): Promise<IBotSecretInfo[]>;
	upsertBotSecret(
		botId: string,
		name: string,
		request: IUpsertBotSecretRequest,
	): Promise<IBotSecretInfo>;
	deleteBotSecret(botId: string, name: string): Promise<void>;
}
