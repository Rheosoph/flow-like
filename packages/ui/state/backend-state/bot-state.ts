import type { BotAppearance } from "../../components/bots/types";
import type { IDataSensitivity } from "../../lib/data-classification";

/** Wire ids, in the order the settings list them. New bots start with `memory`; the rest stays off until switched on. */
export const BOT_CAPABILITIES = [
	"internet",
	"web_requests",
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
	"barcodes",
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
	"user_context",
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

/** A managed device has no key vault, key routes or sealed replies for end-to-end encryption yet. */
export const DEVICE_UNAVAILABLE_BOT_CAPABILITIES: readonly BotCapability[] = [
	"encryption",
];

export function isBotCapabilityAvailable(
	capability: BotCapability,
	location: BotLocation,
): boolean {
	if (location === "server") {
		return !LOCAL_ONLY_BOT_CAPABILITIES.includes(capability);
	}
	if (location === "device") {
		return !DEVICE_UNAVAILABLE_BOT_CAPABILITIES.includes(capability);
	}
	return true;
}

export function botCapabilityUnavailableReason(
	capability: BotCapability,
	location: BotLocation,
): "local" | "device" | null {
	if (isBotCapabilityAvailable(capability, location)) return null;
	return location === "device" ? "device" : "local";
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
	/** System One Bit for response decisions; absent or null disables it. */
	decision?: string | null;
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
	/** Bit ids of the speech-to-text, text-to-speech, image generation and vision models it pins; unset lets the host choose. */
	stt_model?: string | null;
	tts_model?: string | null;
	image_model?: string | null;
	vision_model?: string | null;
	/** Bit id of the model its sub-tasks run on; unset uses the bot's own. */
	subtask_model?: string | null;
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
	stt_model?: string | null;
	tts_model?: string | null;
	image_model?: string | null;
	vision_model?: string | null;
	/** Bit id of the model its sub-tasks run on; unset uses the bot's own. */
	subtask_model?: string | null;
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
	stt_model?: string | null;
	tts_model?: string | null;
	image_model?: string | null;
	vision_model?: string | null;
	/** Bit id of the model its sub-tasks run on; unset uses the bot's own. */
	subtask_model?: string | null;
	classification?: IDataSensitivity;
	capabilities?: BotCapability[];
	profile_id?: string | null;
	location?: BotLocation;
	device_id?: string | null;
}

export type BotPermission =
	| "read_files"
	| "write_files"
	| "read_database"
	| "write_database"
	| "invoke_api"
	| "read_team"
	| "read_boards"
	| "execute_boards"
	| "write_boards"
	| "list_events"
	| "read_events"
	| "execute_events"
	| "write_events"
	| "read_logs"
	| "read_config"
	| "read_templates"
	| "write_templates"
	| "read_courses"
	| "write_courses"
	| "read_widgets"
	| "write_widgets";

export type BotRestrictionPreset =
	| "read_only"
	| "read_and_call_events"
	| "full";

/** `null` does not narrow; an empty list allows nothing. */
export interface IBotScopes {
	files: { prefixes: string[]; read_only: boolean };
	tables?: string[] | null;
	events?: string[] | null;
	boards?: string[] | null;
}

/** What the owner lets a bot do in one app, in every chat. */
export interface IBotRestrictions {
	/** `null` keeps the owner's whole role. */
	permissions: BotPermission[] | null;
	scopes: IBotScopes;
}

export interface IBotApp {
	app_id: string;
	added_at: string;
	/** The owner's current role in the app; null once the owner is no longer a member. */
	owner_role: string | null;
	restrictions?: IBotRestrictions;
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
	/** When the grant lapses unless the bot uses it first (90 days after its last use). */
	expires_at?: string | null;
}

export type BotTurnStatus =
	| "queued"
	| "running"
	| "waiting"
	| "done"
	| "failed"
	| "cancelled";

export type BotNoteState = "open" | "closed";

export interface IBotNoteSensitivity {
	classification?: string | null;
	pii: boolean;
}

export interface IBotNoteVersion {
	version: number;
	title: string;
	status?: string | null;
	body: string;
	state: BotNoteState;
	updated_by: string;
	updated_at: number;
}

/** A bot's working note in one conversation; only the bot's owner reads it. */
export interface IBotNote {
	key: string;
	title: string;
	status?: string | null;
	body: string;
	state: BotNoteState;
	version: number;
	sensitivity: IBotNoteSensitivity;
	updated_by: string;
	created_at: number;
	updated_at: number;
	/** Newest first; only on a single note read. */
	history?: IBotNoteVersion[];
}

/** What a message shows of a note: everyone sees the title and status, never the body. */
export interface IBotNoteRef {
	key: string;
	version: number;
	title: string;
	status?: string | null;
	closed?: boolean;
}

export interface IBotNoteWrite {
	title?: string;
	status?: string;
	body?: string;
	state?: BotNoteState;
	expected_version?: number;
}

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
	/** What the bot says it is doing right now. */
	status_text?: string | null;
	updated_at?: string;
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

/** Positions are fractions of the associated image, before or after resizing. */
export interface IBotInteractionPoint {
	x: number;
	y: number;
}

export interface IBotInteractionRect extends IBotInteractionPoint {
	width: number;
	height: number;
}

export interface IBotInteractionObservation {
	captured_at_ms: number;
	width: number;
	height: number;
	pointer: {
		position: IBotInteractionPoint;
		source: "observed" | "dispatched";
	} | null;
	focus: { label: string; bounds: IBotInteractionRect | null } | null;
	action: {
		/** Action name only; never typed text or key values. */
		kind: string;
		phase: "running" | "completed" | "failed";
		at_ms: number;
		target: IBotInteractionPoint | null;
		target_bounds: IBotInteractionRect | null;
	} | null;
	mapping: { x: number; y: number; width: number; height: number } | null;
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
	observation?: IBotInteractionObservation | null;
	/** The owner has paused the bot's computer calls. */
	taken_over?: boolean;
	/** The owner can drive the picture with mouse and keyboard (device bots with a screen). */
	can_control?: boolean;
	/** Pixels of `image`; input positions are in these. */
	width?: number | null;
	height?: number | null;
	/** A refusal since the last read, such as input the device could not apply. */
	notice?: string | null;
}

/** One mouse or keyboard event for a taken-over screen; positions are pixels of the picture. */
export type IBotInputEvent =
	| { type: "pointer_move"; x: number; y: number }
	| {
			type: "pointer_down" | "pointer_up";
			x: number;
			y: number;
			button: "left" | "middle" | "right";
	  }
	| { type: "scroll"; x: number; y: number; dx: number; dy: number }
	| { type: "key"; chord: string }
	| { type: "text"; text: string };

/**
 * Desktop only: the browser and screen of the bots that run here, and of device bots over the
 * encrypted device tunnel (pass the bot's `device_id`). A local bot's owner uses their own mouse
 * and keyboard while taken over; only device bots take `input`.
 */
export interface IBotComputerHost {
	view(botId: string, deviceId?: string | null): Promise<IBotComputerView>;
	/** Closes the bot's browser and stops the mouse and keyboard action in flight. */
	stop(botId: string, deviceId?: string | null): Promise<void>;
	/** Pauses the bot's computer calls; they fail with `taken_over` until `handBack`. */
	takeOver(botId: string, deviceId?: string | null): Promise<void>;
	handBack(botId: string, deviceId?: string | null): Promise<void>;
	input(
		botId: string,
		event: IBotInputEvent,
		deviceId?: string | null,
	): Promise<void>;
}

/** A bot's own hidden app: its files and databases (and, with the memory phase, its memory). */
export interface IBotWorkspace {
	app_id: string;
}

/** The picture of a bot's current look, which push notifications show as its avatar. */
export interface IBotAvatar {
	/** Null until a picture of the current look was uploaded. */
	url: string | null;
}

export type BotConnectorKind = "mcp" | "oauth";
export type BotConnectorPolicy = "read" | "ask" | "never";
export type BotConnectorAuth =
	| { type: "none" }
	| { type: "secret"; name: string }
	| { type: "oauth"; provider_id: string };

/** One tool of an MCP server as the last refresh saw it. */
export interface IBotConnectorTool {
	name: string;
	description: string;
	read_only_hint?: boolean | null;
	destructive_hint?: boolean | null;
	schema_hash: string;
}

/** An MCP server or OAuth account attached to a bot. No response carries a sign-in. */
export interface IBotConnector {
	id: string;
	bot_id: string;
	kind: BotConnectorKind;
	label: string;
	provider_id?: string | null;
	url?: string | null;
	auth: BotConnectorAuth;
	has_grant: boolean;
	scopes: string[];
	default_policy: BotConnectorPolicy;
	/** Tool name (`read` / `write` for an account) to policy. */
	tool_policies: Record<string, BotConnectorPolicy>;
	tools_snapshot: IBotConnectorTool[];
	/** Each tool's `schema_hash` when the owner last set or confirmed its policy. */
	confirmed_hashes?: Record<string, string>;
	updated_at: string;
}

export interface ICreateBotConnectorRequest {
	kind: BotConnectorKind;
	label: string;
	url?: string;
	auth?: BotConnectorAuth;
	/** The stored sign-in handle of an account (`fl_oauth_…`), never a token. */
	grant?: string;
	scopes?: string[];
	default_policy?: BotConnectorPolicy;
}

export interface IUpdateBotConnectorRequest {
	label?: string;
	default_policy?: BotConnectorPolicy;
	tool_policies?: Record<string, BotConnectorPolicy>;
	grant?: string;
	scopes?: string[];
	/** Tools whose changed schema the owner has looked at and accepts. */
	confirm_tools?: string[];
}

/** An account the person connected, as the picker lists it. */
export interface IConnectedAccount {
	/** Stored on the connector as `grant`; empty where the device keeps the sign-in itself. */
	handle: string;
	provider_id: string | null;
	scopes: string[];
	state: string;
}

/** A bot webhook's metadata; its stored path does not contain the secret. */
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
	/** Send in X-Flow-Like-Webhook-Secret, as a bearer token, or in the secret URL parameter. */
	secret: string;
	/** Optional key for `X-Flow-Like-Signature: sha256=<hmac of the body>`. */
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

/** One mail thread of a bot; its mails live in the owner's chat with the bot. */
export interface IBotMailThread {
	id: string;
	subject: string;
	participants: string[];
	last_message_at: string;
	last_snippet?: string | null;
	message_count: number;
	/** Received mails newer than the owner's read marker of the chat holding them. */
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
	monitoring?: boolean;
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
	/** The group or chat the task fires in; none fires in the owner's chat with the bot. */
	conversation?: IBotTaskConversation | null;
}

/** Where a task scheduled outside the owner's chat fires. */
export interface IBotTaskConversation {
	id: string;
	kind: "group" | "direct";
	/** The group's name, when it has one. */
	title?: string | null;
}

export interface ICreateBotTaskRequest {
	monitoring?: boolean;
	kind: "once" | "cron";
	at?: string;
	cron?: string;
	tz?: string;
	instruction: string;
	title?: string;
}

export interface IUpdateBotTaskRequest {
	monitoring?: boolean;
	at?: string;
	cron?: string;
	tz?: string;
	instruction?: string;
	title?: string;
	/** Pausing keeps the task; resuming computes its next run from now. */
	enabled?: boolean;
}

export type BotSkillStatus = "active" | "pending" | "disabled";

/** Guidance the owner installed on a bot. Only an active skill reaches the bot. */
export interface IBotSkill {
	id: string;
	bot_id: string;
	/** Lowercase letters, digits and single hyphens. */
	name: string;
	description: string;
	/** Markdown instructions. */
	body: string;
	/** `owner`, `import`, `bot_proposed` or `package:<id>@<version>`. */
	source: string;
	status: BotSkillStatus;
	/** Package node names the skill's instructions call. */
	tools: string[];
	updated_at?: string;
}

export interface ICreateBotSkillRequest {
	name: string;
	description: string;
	body: string;
	tools?: string[];
}

export interface IUpdateBotSkillRequest {
	description?: string;
	body?: string;
	tools?: string[];
}

export interface IBotState {
	/** Recent turns across this person's hosts, for mobile status surfaces. */
	getMobileStatus?(): Promise<IBotTurn[]>;
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
	/** Skills of a bot with the skills capability, by name; absent where skills are not kept. */
	listSkills?(botId: string): Promise<IBotSkill[]>;
	/** A skill the person wrote: active at once. */
	createSkill?(
		botId: string,
		request: ICreateBotSkillRequest,
	): Promise<IBotSkill>;
	updateSkill?(
		botId: string,
		skillId: string,
		request: IUpdateBotSkillRequest,
	): Promise<IBotSkill>;
	/** Pending or disabled to active: the bot sees it from its next turn. */
	approveSkill?(botId: string, skillId: string): Promise<IBotSkill>;
	disableSkill?(botId: string, skillId: string): Promise<IBotSkill>;
	deleteSkill?(botId: string, skillId: string): Promise<void>;
	/**
	 * Pins a registry package to the bot's workspace and saves the skills it ships as pending.
	 * Refused (409) when the package needs capabilities the bot does not have.
	 */
	installSkillPackage?(
		botId: string,
		packageId: string,
		version?: string,
	): Promise<IBotSkill[]>;
	/** A zipped skill folder (SKILL.md plus references); the skill starts pending. */
	importSkill?(botId: string, zip: Uint8Array): Promise<IBotSkill>;
	/** MCP servers and accounts attached to the bot. */
	listConnectors?(botId: string): Promise<IBotConnector[]>;
	createConnector?(
		botId: string,
		request: ICreateBotConnectorRequest,
	): Promise<IBotConnector>;
	updateConnector?(
		botId: string,
		connectorId: string,
		request: IUpdateBotConnectorRequest,
	): Promise<IBotConnector>;
	deleteConnector?(botId: string, connectorId: string): Promise<void>;
	/** Asks the MCP server for its tools and keeps the list with the connector. */
	refreshConnector?(botId: string, connectorId: string): Promise<IBotConnector>;
	/** The accounts the person connected, to attach to a bot. */
	listConnectedAccounts?(): Promise<IConnectedAccount[]>;
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
	/** The bot's notes in one conversation, without history; owner only. */
	listNotes?(botId: string, conversationId: string): Promise<IBotNote[]>;
	/** With history. */
	getNote?(
		botId: string,
		conversationId: string,
		key: string,
	): Promise<IBotNote>;
	/** Rejects with a 409 error when `expected_version` is stale. */
	updateNote?(
		botId: string,
		conversationId: string,
		key: string,
		write: IBotNoteWrite,
	): Promise<IBotNote>;
	/** The picture of an account bot's current look; absent where nothing pushes notifications. */
	getAvatar?(botId: string): Promise<IBotAvatar>;
	/** Uploads `image`, a data URL of `appearance`; refused when the bot looks different now. */
	putAvatar?(
		botId: string,
		appearance: BotAppearance,
		image: string,
	): Promise<IBotAvatar>;
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
	setBotAppRestrictions?(
		botId: string,
		appId: string,
		restrictions: IBotRestrictions,
	): Promise<IBotRestrictions>;
	listBotSecrets(botId: string): Promise<IBotSecretInfo[]>;
	upsertBotSecret(
		botId: string,
		name: string,
		request: IUpsertBotSecretRequest,
	): Promise<IBotSecretInfo>;
	deleteBotSecret(botId: string, name: string): Promise<void>;
}
