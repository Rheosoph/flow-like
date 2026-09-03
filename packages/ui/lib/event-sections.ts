import type { IEvent } from "./schema/flow/event";

/**
 * Section + guidance descriptors for the event configuration surface.
 *
 * The event detail screen is generated from these: the rail is `getEventSections`,
 * the checklist is `getEventGuide`, and the callout above each section is
 * `getSectionGuidance`. Adding an event type means adding entries here, not
 * touching the layout.
 */

/**
 * Shared sections exist on every event. Type-specific sections use their own
 * ids (`connection`, `permissions`, …) and are declared in TRIGGER_SECTIONS,
 * which is why this is a string rather than a closed union.
 */
export type EventSectionId = string;

export const SHARED_SECTION_IDS = [
	"flow",
	"inputs",
	"variables",
	"release",
	"canary",
	"quality",
	"history",
	"identity",
] as const;

/** True for sections whose content comes from the event type's config component. */
export function isTriggerSection(id: EventSectionId): boolean {
	return !(SHARED_SECTION_IDS as readonly string[]).includes(id);
}

export interface IEventSection {
	id: EventSectionId;
	label: string;
	/** Lucide icon name, resolved by the rail component. */
	icon: string;
	blurb: string;
}

export interface IEventGuideStep {
	id: string;
	title: string;
	/** Why it matters — shown under the title, so keep it to one sentence. */
	why: string;
	/** Section to jump to. Omit for steps that happen outside this screen. */
	section?: EventSectionId;
	/** Marks work done in another product (a developer portal, a mail provider). */
	external?: boolean;
	/** Where to do it, when `external`. */
	where?: string;
	/**
	 * Derives the tick from the saved config. Steps without it are
	 * user-confirmed, because nothing on this screen can prove them.
	 */
	auto?: (config: Record<string, unknown>, event: IEvent) => boolean;
}

export interface ISectionGuidance {
	/**
	 * Extra context. Rendered only when it differs from the section's own blurb,
	 * because the section header already answers "what is this for".
	 */
	what?: string;
	/** The failure people actually hit. This is the support burden, written down. */
	mistake: string;
}

const filled = (value: unknown): boolean => {
	if (value === null || value === undefined) return false;
	if (typeof value === "string") return value.trim().length > 0;
	if (Array.isArray(value)) return value.length > 0;
	if (typeof value === "number") return true;
	if (typeof value === "boolean") return value;
	return Object.keys(value as object).length > 0;
};

const has =
	(key: string) =>
	(config: Record<string, unknown>): boolean =>
		filled(config?.[key]);

/**
 * Event types whose config component understands `section` and renders only
 * that slice. Everything else gets the single "trigger" section below and
 * renders whole, so splitting a component is opt-in.
 */
const TRIGGER_SECTIONS: Record<string, IEventSection[]> = {
	discord: [
		{
			id: "connection",
			label: "Connection",
			icon: "plug",
			blurb:
				"Credentials and transport. Gateway holds an open socket on this device; interactions is a webhook the server can answer.",
		},
		{
			id: "permissions",
			label: "Permissions",
			icon: "shield",
			blurb:
				"Gateway intents decide which events Discord streams to you. Ask for the minimum.",
		},
		{
			id: "channels",
			label: "Channel filters",
			icon: "hash",
			blurb: "Empty lists mean the bot listens everywhere it has been invited.",
		},
		{
			id: "behaviour",
			label: "Bot behaviour",
			icon: "message-square",
			blurb:
				"How the bot decides a message is meant for it, and how it introduces itself.",
		},
	],
	cron: [
		{
			id: "schedule",
			label: "Schedule",
			icon: "clock",
			blurb:
				"When this fires. The presets, the expression and the guided builder are three views of one value.",
		},
		{
			id: "runtime",
			label: "Runtime",
			icon: "server",
			blurb: "Where the schedule executes and against which clock.",
		},
	],
	telegram: [
		{
			id: "connection",
			label: "Connection",
			icon: "plug",
			blurb:
				"The bot token, and whether updates arrive by webhook or by polling.",
		},
		{
			id: "chats",
			label: "Chat filters",
			icon: "hash",
			blurb: "Which chats the bot will answer in.",
		},
		{
			id: "behaviour",
			label: "Bot behaviour",
			icon: "message-square",
			blurb: "Identity, and how the bot decides a message is meant for it.",
		},
	],
	api: [
		{
			id: "endpoint",
			label: "Endpoint",
			icon: "globe",
			blurb: "Method, path and the URL callers actually hit.",
		},
		{
			id: "access",
			label: "Access",
			icon: "shield",
			blurb: "Who is allowed to call this endpoint.",
		},
	],
	simple_chat: [
		{
			id: "appearance",
			label: "Appearance",
			icon: "brush",
			blurb:
				"What users see before they type, and where the chat can navigate to.",
		},
		{
			id: "capabilities",
			label: "Capabilities",
			icon: "zap",
			blurb: "What users can send, and how much context comes with it.",
		},
		{
			id: "voice",
			label: "Voice",
			icon: "mic",
			blurb: "Voice input, playback and how the recorder presents itself.",
		},
		{
			id: "tools",
			label: "Tools & starters",
			icon: "wrench",
			blurb: "What the assistant may call, and how people get going.",
		},
	],
	daemon: [
		{
			id: "supervision",
			label: "Supervision",
			icon: "server",
			blurb:
				"Restart policy and the backoff window that keeps a crash loop sane.",
		},
		{
			id: "logging",
			label: "Polling & logs",
			icon: "form-input",
			blurb:
				"How often the daemon checks for work, and how it batches its logs.",
		},
	],
	rest: [
		{
			id: "server",
			label: "Server",
			icon: "server",
			blurb: "Base URL, the generated OpenAPI surface and the public alias.",
		},
		{
			id: "routes",
			label: "Registered routes",
			icon: "layers",
			blurb: "What the workflow declared, and the auth each route expects.",
		},
	],
	mcp: [
		{
			id: "server",
			label: "Server",
			icon: "plug",
			blurb: "The endpoint AI clients connect to, and how they authenticate.",
		},
		{
			id: "registry",
			label: "Registry",
			icon: "wrench",
			blurb: "The tools, resources and prompts this server exposes.",
		},
		{
			id: "inspector",
			label: "Live inspector",
			icon: "zap",
			blurb: "Calls the server the way a real MCP client would.",
		},
	],
};

const TRIGGER_LABELS: Record<
	string,
	{ label: string; icon: string; blurb: string }
> = {
	cron: {
		label: "Schedule",
		icon: "clock",
		blurb: "When this fires, in which timezone, and where it runs.",
	},
	api: {
		label: "Endpoint",
		icon: "globe",
		blurb:
			"The URL callers hit, who may call it, and how to reach it from outside.",
	},
	rest: {
		label: "REST server",
		icon: "server",
		blurb: "Base URL, registered routes, authentication and public aliases.",
	},
	mcp: {
		label: "MCP server",
		icon: "plug",
		blurb: "The endpoint AI clients connect to, and everything it exposes.",
	},
	simple_chat: {
		label: "Chat surface",
		icon: "message-square",
		blurb: "Appearance, capabilities, voice, tools and starter prompts.",
	},
	discord: {
		label: "Discord bot",
		icon: "hash",
		blurb: "Credentials, gateway intents, channel filters and behaviour.",
	},
	telegram: {
		label: "Telegram bot",
		icon: "send",
		blurb: "Credentials, chat filters and behaviour.",
	},
	email: {
		label: "Mailbox",
		icon: "mail",
		blurb: "The account, the IMAP connection and which messages start a run.",
	},
	daemon: {
		label: "Supervision",
		icon: "server",
		blurb: "Restart policy, backoff window and log batching.",
	},
	deeplink: {
		label: "Deep link",
		icon: "link",
		blurb: "The route that opens this event from outside the app.",
	},
	quick_action: {
		label: "Quick action",
		icon: "zap",
		blurb: "How this appears when run from the command palette.",
	},
	generic_form: {
		label: "Form",
		icon: "clipboard-list",
		blurb: "The generated input form and where it navigates afterwards.",
	},
	page: {
		label: "Page",
		icon: "layout",
		blurb: "The page this event renders and the route it lives on.",
	},
};

const DEFAULT_TRIGGER = {
	label: "Configuration",
	icon: "cog",
	blurb: "Settings specific to this kind of event.",
};

/**
 * The icon and short label that identify an event's kind in a list. Shares
 * TRIGGER_LABELS with the section rail so a Discord event is a hash everywhere.
 */
export function getEventTypeGlyph(event: IEvent): {
	label: string;
	icon: string;
} {
	const entry = event.default_page_id
		? TRIGGER_LABELS.page
		: (TRIGGER_LABELS[event.event_type] ?? DEFAULT_TRIGGER);
	return { label: entry.label, icon: entry.icon };
}

const SHARED_SECTIONS: IEventSection[] = [
	{
		id: "flow",
		label: "Flow & target",
		icon: "layers",
		blurb: "What runs when this fires, and which snapshot of it.",
	},
	{
		id: "inputs",
		label: "Inputs",
		icon: "form-input",
		blurb:
			"Input pins captured when this event was published. Drift against the node is shown here.",
	},
	{
		id: "variables",
		label: "Variable overrides",
		icon: "code",
		blurb:
			"Flow variables this event overrides at run time. Variables marked exposed, secret or runtime-configured on the flow can be overridden — runtime-configured ones have to be set here for triggers that run without a user.",
	},
	{
		id: "release",
		label: "Release",
		icon: "git-branch",
		blurb: "Versioning and the record of what changed.",
	},
	{
		id: "canary",
		label: "Canary",
		icon: "split",
		blurb:
			"Send a share of this event's traffic to another flow or version before promoting it.",
	},
	{
		id: "quality",
		label: "Quality",
		icon: "flask-conical",
		blurb:
			"Replay recorded real inputs against a candidate version and catch regressions before they ship.",
	},
	{
		id: "history",
		label: "History",
		icon: "history",
		blurb:
			"Every version this event has shipped, and how each one has been running.",
	},
	{
		id: "identity",
		label: "Identity",
		icon: "file-text",
		blurb: "How this event is named, described and correlated.",
	},
];

export function getEventSections(event: IEvent): IEventSection[] {
	// Page-target events have no type-specific config component — there is no
	// `configInterfaces["page"]` — and their page/version fields live in the
	// shared Flow & target section. Giving them a trigger section produces a tab
	// that can never render anything. Quality is dropped too: page payloads are
	// sealed to their page session, so regression suites exclude page events.
	if (event.default_page_id)
		return SHARED_SECTIONS.filter((section) => section.id !== "quality");
	const split = TRIGGER_SECTIONS[event.event_type];
	if (split) return [...split, ...SHARED_SECTIONS];
	const trigger = TRIGGER_LABELS[event.event_type] ?? DEFAULT_TRIGGER;
	return [{ id: "trigger", ...trigger }, ...SHARED_SECTIONS];
}

/* ------------------------------------------------------------------ guides */

const FLOW_STEPS: IEventGuideStep[] = [
	{
		id: "bind-flow",
		title: "Bind the flow and choose a version",
		why: "Latest follows your edits — fine while building, risky once this matters.",
		section: "flow",
		// A page-target event is bound by its page, not by an entry node.
		auto: (_c, event) =>
			event.default_page_id
				? filled(event.default_page_id)
				: filled(event.board_id) && filled(event.node_id),
	},
	{
		id: "case-keys",
		title: "Add case keys",
		why: "Without them runs never group into a case, so process mining can't follow one business object across apps.",
		section: "identity",
		auto: (_c, event) =>
			Object.keys(event.correlation_mappings ?? {}).length > 0,
	},
];

const EVENT_GUIDES: Record<string, IEventGuideStep[]> = {
	cron: [
		{
			id: "when",
			title: "Choose when it fires",
			why: "The presets, the expression and the guided builder all write the same value.",
			section: "schedule",
			auto: has("expression"),
		},
		{
			id: "timezone",
			title: "Confirm the timezone",
			why: "A named zone follows daylight saving; UTC never shifts. Getting this wrong moves every run by an hour twice a year.",
			section: "schedule",
			auto: has("timezone"),
		},
		{
			id: "where",
			title: "Decide where it runs",
			why: "A closed laptop does not fire schedules. Remote keeps firing.",
			section: "runtime",
			auto: (_c, event) => filled(event.execution_mode),
		},
		...FLOW_STEPS,
		{
			id: "first-run",
			title: "Run it once and read the log",
			why: "A schedule that has never run is a schedule you don't know works.",
		},
	],
	api: [
		{
			id: "route",
			title: "Set the method and path",
			why: "Method and path together must be unique within this app.",
			section: "endpoint",
			auto: has("path"),
		},
		{
			id: "auth",
			title: "Decide who may call it",
			why: "The most consequential setting here — a public endpoint can be triggered by anyone with the URL.",
			section: "access",
			auto: (config) =>
				filled(config?.public_endpoint) || filled(config?.auth_token),
		},
		{
			id: "store-token",
			title: "Store the token somewhere safe",
			why: "Rotating it later invalidates every existing caller immediately.",
		},
		...FLOW_STEPS,
		{
			id: "test-request",
			title: "Send a test request",
			why: "Proves the auth header, the path and the flow binding in one shot.",
			section: "endpoint",
		},
	],
	discord: [
		{
			id: "create-app",
			title: "Create the application in Discord",
			why: "Everything else needs the application to exist first.",
			external: true,
			where: "discord.com/developers/applications",
		},
		{
			id: "token",
			title: "Reset the bot token and paste it here",
			why: "The token is the whole connection — nothing works without it.",
			section: "connection",
			auto: has("token"),
		},
		{
			id: "message-content",
			title: "Enable MESSAGE CONTENT INTENT in the portal",
			why: "The most common reason a bot connects and then replies to nothing: every message arrives empty.",
			external: true,
			where: "Portal → Bot → Privileged Gateway Intents",
		},
		{
			id: "invite",
			title: "Invite the bot to your server",
			why: "Tick bot and Send Messages, then open the generated invite link.",
			external: true,
			where: "Portal → OAuth2 → URL generator",
		},
		{
			id: "channels",
			title: "Pick the channels it listens in",
			why: "An empty whitelist means everywhere it was invited — usually louder than you want.",
			section: "channels",
			auto: has("channel_whitelist"),
		},
		{
			id: "intents",
			title: "Check the gateway intents",
			why: "MessageContent is privileged: enabled here but not in the portal, every message arrives empty.",
			section: "permissions",
			auto: has("intents"),
		},
		...FLOW_STEPS,
		{
			id: "say-hi",
			title: "Send it a message in Discord",
			why: "The only way to confirm the intents, the invite and the flow all line up.",
		},
	],
	telegram: [
		{
			id: "botfather",
			title: "Create the bot with BotFather",
			why: "BotFather issues the token everything else depends on.",
			external: true,
			where: "Telegram → @BotFather → /newbot",
		},
		{
			id: "token",
			title: "Paste the bot token",
			why: "Without it the bot cannot connect or receive updates.",
			section: "connection",
			auto: has("bot_token"),
		},
		{
			id: "scope",
			title: "Decide which chats it answers in",
			why: "An empty whitelist means every chat it has been added to.",
			section: "chats",
			auto: has("chat_whitelist"),
		},
		...FLOW_STEPS,
		{
			id: "say-hi",
			title: "Message the bot",
			why: "Confirms the token, the mode and the flow binding together.",
		},
	],
	simple_chat: [
		{
			id: "disclosure",
			title: "Set the AI disclosure",
			why: "Shown once above the first message, and required in several jurisdictions.",
			section: "appearance",
			auto: has("ai_disclosure"),
		},
		{
			id: "starters",
			title: "Add starter prompts",
			why: "Empty chat boxes get abandoned. Starters are the highest-leverage thing on the screen.",
			section: "tools",
			auto: has("example_messages"),
		},
		{
			id: "capabilities",
			title: "Decide uploads and history",
			why: "Both change the payload the flow receives — uploads add file paths to it.",
			section: "capabilities",
			auto: (config) =>
				config?.allow_file_upload !== undefined ||
				filled(config?.history_elements),
		},
		{
			id: "tools",
			title: "Pick the tools it may call",
			why: "With none selected the flow answers from its own logic only.",
			section: "tools",
			auto: has("tools"),
		},
		...FLOW_STEPS,
		{
			id: "ask",
			title: "Open it and ask a real question",
			why: "Preview shows layout; only a real run shows latency and answer quality.",
		},
	],
	email: [
		{
			id: "address",
			title: "Enter the mailbox address",
			why: "This is both the account read from and the identity replies are sent as.",
			section: "trigger",
			auto: has("mail"),
		},
		{
			id: "app-password",
			title: "Create an app password with your provider",
			why: "Account passwords usually fail against IMAP once two-factor is on.",
			external: true,
			where: "Your mail provider → Security → App passwords",
		},
		{
			id: "imap",
			title: "Fill in the IMAP server and port",
			why: "993 for implicit TLS, 143 for STARTTLS. An MX record is not an IMAP host.",
			section: "trigger",
			auto: has("imap_server"),
		},
		{
			id: "imap-password",
			title: "Save the mailbox password",
			why: "Held in the OS keychain, never in the app config.",
			section: "trigger",
			auto: (config) =>
				filled(config?.secret_imap_password) || filled(config?.password),
		},
		...FLOW_STEPS,
		{
			id: "activate",
			title: "Activate the event",
			why: "Nothing is polled while the event is inactive.",
			auto: (_c, event) => !!event.active,
		},
	],
	daemon: [
		{
			id: "policy",
			title: "Choose a restart policy",
			why: "on_failure restarts only after a crash; always restarts even on a clean exit.",
			section: "supervision",
			auto: has("restart_policy"),
		},
		{
			id: "backoff",
			title: "Set the backoff window",
			why: "Too tight and a crash loop hammers whatever the daemon depends on.",
			section: "supervision",
			auto: has("max_restart_delay_ms"),
		},
		...FLOW_STEPS,
		{
			id: "watch",
			title: "Watch one restart cycle",
			why: "Confirms the backoff and the healthy-reset window behave as you expect.",
		},
	],
	mcp: [
		{
			id: "auth",
			title: "Set authentication",
			why: "An open MCP server lets any client call every tool you registered.",
			section: "server",
		},
		{
			id: "register",
			title: "Register at least one tool",
			why: "A server with no tools initializes fine and then does nothing.",
			section: "registry",
		},
		...FLOW_STEPS,
		{
			id: "inspect",
			title: "Run initialize in the inspector",
			why: "Confirms auth, protocol version and the tool list in one call.",
			section: "inspector",
		},
		{
			id: "add-client",
			title: "Add the server to your AI client",
			why: "Paste the base URL and the token into the client's MCP configuration.",
			external: true,
			where: "Client → Settings → MCP servers",
		},
	],
	rest: [
		{
			id: "auth",
			title: "Set authentication",
			why: "An unauthenticated REST surface is callable by anyone who finds the base URL.",
			section: "server",
		},
		{
			id: "routes",
			title: "Register your routes",
			why: "The server serves nothing until at least one route is registered.",
			section: "routes",
		},
		...FLOW_STEPS,
		{
			id: "spec",
			title: "Check the generated OpenAPI spec",
			why: "It is what consumers will generate their clients from.",
			section: "server",
		},
	],
	deeplink: [
		{
			id: "route",
			title: "Choose the route",
			why: "This is the path after flow-like:// that opens the event.",
			section: "trigger",
			auto: has("route"),
		},
		...FLOW_STEPS,
		{
			id: "try",
			title: "Open the link once",
			why: "Deep links only work on desktop, and only once the app has registered the scheme.",
		},
	],
};

const GENERIC_GUIDE: IEventGuideStep[] = [
	...FLOW_STEPS,
	{
		id: "run-once",
		title: "Run it once",
		why: "The fastest way to find out whether the payload and the flow agree.",
	},
];

export function getEventGuide(event: IEvent): IEventGuideStep[] {
	return EVENT_GUIDES[event.event_type] ?? GENERIC_GUIDE;
}

/* -------------------------------------------------------------- guidance */

const SHARED_GUIDANCE: Record<string, ISectionGuidance> = {
	flow: {
		what: "What runs when this fires, and which snapshot of it.",
		mistake:
			"Pinning a version and forgetting: the flow evolves and the event keeps running the old snapshot.",
	},
	inputs: {
		what: "The payload shape captured when this event was published.",
		mistake:
			"Ignoring drift. The node gains a pin, the event keeps sending the old payload, and the flow reads nothing.",
	},
	variables: {
		what: "Values this event injects over the flow's own defaults.",
		mistake:
			"Trying to override a variable that isn't marked exposed on the flow — it silently does nothing.",
	},
	release: {
		what: "Versioning and the record of what changed.",
		mistake:
			"Shipping a behaviour change with no note, then reconstructing it from run logs later.",
	},
	canary: {
		what: "A weighted split between the live target and one or two candidate targets.",
		mistake:
			"Reading a quiet stats table as a healthy canary: at low weights it takes many triggers before errors show, so give it traffic and time before promoting.",
	},
	quality: {
		what: "A regression suite built from recorded real inputs and the board's authored test* events, replayed against a candidate version.",
		mistake:
			"Reading a green suite as full coverage: without Assert nodes it only proves the replays didn't error — and replays still execute live side effects like outbound HTTP.",
	},
	history: {
		what: "Past versions of this event, and what their runs actually did.",
		mistake:
			"Trusting run counts for old versions: runs recorded before version stamping carry no version key and group separately as unversioned.",
	},
	identity: {
		what: "How this event is named, described and correlated.",
		mistake:
			"Skipping case keys, which means runs never group into a business case.",
	},
};

/**
 * Page-target overrides for shared sections. Page canaries resolve once at
 * bootstrap and the sealed page claims pin the session, so the dispatch-style
 * "traffic share" mental model misleads here.
 */
const PAGE_GUIDANCE: Record<string, ISectionGuidance> = {
	canary: {
		what: "A weighted split between the primary page and one or two candidate pages, assigned per viewer when the page bootstraps.",
		mistake:
			"Expecting a weight change to move viewers already on the page: a session keeps the variant it bootstrapped into until it reloads, so give new sessions time before reading the stats.",
	},
};

const TRIGGER_GUIDANCE: Record<string, ISectionGuidance> = {
	cron: {
		what: "Decides when the flow runs, and against which clock.",
		mistake:
			"Writing a six-field expression: with six fields the first one is seconds, so the schedule lands somewhere you didn't intend.",
	},
	api: {
		what: "The URL callers hit and what you accept from them.",
		mistake: "Turning on Public to test quickly and never turning it back off.",
	},
	discord: {
		what: "Credentials, permissions and where the bot listens.",
		mistake:
			"Enabling MessageContent here but not in the developer portal. The bot connects, and every message arrives empty.",
	},
	telegram: {
		what: "Credentials, chat filters and behaviour.",
		mistake:
			"Leaving a webhook registered while switching to polling mode — updates go to the old URL.",
	},
	simple_chat: {
		what: "What users see, what they can send, and what the assistant may call.",
		mistake:
			"Enabling voice input but leaving playback on Text. People speak, hear nothing back, and read it as broken.",
	},
	email: {
		what: "The mailbox, its credentials and which messages are worth a run.",
		mistake:
			"No filter at all: every newsletter in the folder becomes a flow run.",
	},
	daemon: {
		what: "How the long-running process is kept alive.",
		mistake:
			"A one-second backoff ceiling, which turns a dependency outage into a crash loop.",
	},
	mcp: {
		what: "The MCP endpoint and everything it exposes to AI clients.",
		mistake:
			"Registering a tool without argument descriptions — the model then guesses the arguments.",
	},
	rest: {
		what: "The REST surface, its routes and its authentication.",
		mistake:
			"Renaming the public alias after consumers have generated clients against it.",
	},
	deeplink: {
		what: "The route that opens this event from outside the app.",
		mistake:
			"Assuming it works on mobile web — deep links only resolve on desktop.",
	},
};

/** Guidance for the split sections, keyed `${event_type}.${section}`. */
const SPLIT_GUIDANCE: Record<string, ISectionGuidance> = {
	"discord.connection": {
		what: "Credentials and how the bot reaches Discord.",
		mistake:
			"Pasting the application ID where the token belongs — both are long and look alike.",
	},
	"discord.permissions": {
		what: "Which events Discord will stream to this bot.",
		mistake:
			"Enabling MessageContent here but not in the developer portal. The bot connects, and every message arrives empty.",
	},
	"discord.channels": {
		what: "Where the bot listens.",
		mistake:
			"Leaving both lists empty and being surprised it answers everywhere it was invited.",
	},
	"discord.behaviour": {
		what: "How the bot decides a message is meant for it.",
		mistake:
			"Turning mention-only off in a busy channel, which is how a bot gets muted.",
	},
	"cron.schedule": {
		what: "Decides when the flow runs.",
		mistake:
			"Writing a six-field expression: with six fields the first one is seconds, so the schedule lands somewhere you didn't intend.",
	},
	"cron.runtime": {
		what: "Where the schedule executes and against which clock.",
		mistake:
			"Leaving it local and expecting overnight runs from a closed laptop.",
	},
	"telegram.connection": {
		what: "The token, and how updates reach you.",
		mistake:
			"Switching to polling without deleting the webhook first — updates keep going to the old URL and the bot looks dead.",
	},
	"telegram.chats": {
		what: "Which chats the bot answers in.",
		mistake:
			"Using a group name instead of the numeric chat ID, which never matches.",
	},
	"telegram.behaviour": {
		what: "Identity, and what counts as talking to the bot.",
		mistake:
			"Leaving mention-only off in a busy group, so the bot replies to everything.",
	},
	"api.endpoint": {
		what: "The URL callers hit.",
		mistake:
			"Two events sharing an app ID, path and method — only the most recently registered one fires.",
	},
	"api.access": {
		what: "Who is allowed to trigger this flow from outside.",
		mistake: "Adding a token to test, then never turning off the public path.",
	},
	"simple_chat.appearance": {
		what: "What users see before they type anything.",
		mistake:
			"Writing custom CSS against the app's tokens instead of the --fl-chat-* ones, so nothing changes.",
	},
	"simple_chat.capabilities": {
		what: "What users can send, and how much history rides along.",
		mistake:
			"Turning on uploads without wiring the file pins in the flow, so attachments silently vanish.",
	},
	"simple_chat.voice": {
		what: "Voice input, playback and the recorder's appearance.",
		mistake:
			"Enabling voice input but leaving playback on Text. People speak, hear nothing back, and read it as broken.",
	},
	"simple_chat.tools": {
		what: "What the assistant may call, and how people start.",
		mistake:
			"Adding a tool to Available but not to Default, then wondering why it is never used.",
	},
	"daemon.supervision": {
		what: "How the process is kept alive.",
		mistake:
			"A backoff ceiling of a second or two, which turns a dependency outage into a hot crash loop.",
	},
	"daemon.logging": {
		what: "Polling cadence and log batching.",
		mistake:
			"A very short board poll interval, which costs more than it saves on a board that rarely changes.",
	},
	"rest.server": {
		what: "The REST surface and its public identity.",
		mistake:
			"Renaming the public alias after consumers have generated clients against it.",
	},
	"rest.routes": {
		what: "What the workflow declared, and what each route expects.",
		mistake:
			"Expecting routes to appear before saving — setup runs on save, not on edit.",
	},
	"mcp.server": {
		what: "The endpoint AI clients connect to.",
		mistake:
			"Leaving it unauthenticated, which lets any client call every tool you registered.",
	},
	"mcp.registry": {
		what: "Everything this server exposes.",
		mistake:
			"Registering a tool without argument descriptions — the model then guesses the arguments.",
	},
	"mcp.inspector": {
		what: "Calls the server as a real client would.",
		mistake:
			"Testing with a different token than the one clients will actually use.",
	},
};

export function getSectionGuidance(
	event: IEvent,
	sectionId: EventSectionId,
): ISectionGuidance | null {
	const split = SPLIT_GUIDANCE[`${event.event_type}.${sectionId}`];
	if (split) return split;
	if (sectionId === "trigger") {
		return TRIGGER_GUIDANCE[event.event_type] ?? null;
	}
	if (event.default_page_id) {
		const page = PAGE_GUIDANCE[sectionId];
		if (page) return page;
	}
	return SHARED_GUIDANCE[sectionId] ?? null;
}
