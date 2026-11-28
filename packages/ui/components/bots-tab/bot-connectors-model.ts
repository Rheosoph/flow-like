import type {
	BotConnectorPolicy,
	IBotConnector,
	IBotConnectorCalendar,
	IBotConnectorTool,
	IConnectedAccount,
	ICreateBotConnectorRequest,
} from "../../state/backend-state/bot-state";

export const CONNECTOR_POLICIES: readonly BotConnectorPolicy[] = [
	"read",
	"ask",
	"never",
];

/** What a tool does without an explicit choice: the server's hints, else the connector's default. */
export function defaultToolPolicy(
	tool: IBotConnectorTool,
	connectorDefault: BotConnectorPolicy,
): BotConnectorPolicy {
	if (tool.destructive_hint) return "ask";
	if (tool.read_only_hint) return "read";
	return connectorDefault;
}

/** The policy the bot runs with for one tool, and whether the owner chose it. */
export function toolPolicy(
	connector: Pick<IBotConnector, "tool_policies" | "default_policy">,
	tool: IBotConnectorTool,
): { policy: BotConnectorPolicy; chosen: boolean } {
	const chosen = connector.tool_policies[tool.name];
	if (chosen) return { policy: chosen, chosen: true };
	return {
		policy: defaultToolPolicy(tool, connector.default_policy),
		chosen: false,
	};
}

/** The tool's schema changed since the owner set or confirmed its policy: it asks every time. */
export function isDrifted(
	connector: Pick<IBotConnector, "confirmed_hashes">,
	tool: IBotConnectorTool,
): boolean {
	const confirmed = connector.confirmed_hashes?.[tool.name];
	return Boolean(
		confirmed && tool.schema_hash && confirmed !== tool.schema_hash,
	);
}

/** A tool that changes things must not run unasked without the owner knowing it. */
export function readsWriteTool(
	tool: IBotConnectorTool,
	policy: BotConnectorPolicy,
): boolean {
	return policy === "read" && !tool.read_only_hint;
}

/** Account connectors have two pseudo tools: GET and HEAD, and every other method. */
export const ACCOUNT_POLICY_KEYS = ["read", "write"] as const;

export function accountPolicy(
	connector: Pick<IBotConnector, "tool_policies">,
	key: (typeof ACCOUNT_POLICY_KEYS)[number],
): { policy: BotConnectorPolicy; chosen: boolean } {
	const chosen = connector.tool_policies[key];
	if (chosen) return { policy: chosen, chosen: true };
	return { policy: key === "read" ? "read" : "ask", chosen: false };
}

/** The policies after the owner changed one; choosing the default again removes the entry. */
export function withToolPolicy(
	current: Record<string, BotConnectorPolicy>,
	name: string,
	policy: BotConnectorPolicy,
): Record<string, BotConnectorPolicy> {
	return { ...current, [name]: policy };
}

/** Wire format of an MCP URL typed by the owner: trimmed, with a scheme. */
export function normalizeMcpUrl(input: string): string {
	const trimmed = input.trim();
	if (!trimmed) return "";
	return /^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed)
		? trimmed
		: `https://${trimmed}`;
}

export type CalDavPresetId =
	| "icloud"
	| "nextcloud"
	| "fastmail"
	| "mailbox"
	| "posteo"
	| "synology"
	| "radicale"
	| "other";

export interface CalDavPreset {
	id: CalDavPresetId;
	name: string;
	/** Prefilled address; `<…>` marks the part the owner must replace. */
	address: string;
}

export const CALDAV_PRESETS: readonly CalDavPreset[] = [
	{ id: "icloud", name: "iCloud", address: "https://caldav.icloud.com/" },
	{
		id: "nextcloud",
		name: "Nextcloud",
		address: "https://<host>/remote.php/dav/",
	},
	{ id: "fastmail", name: "Fastmail", address: "https://caldav.fastmail.com/" },
	{ id: "mailbox", name: "mailbox.org", address: "https://dav.mailbox.org/" },
	{ id: "posteo", name: "Posteo", address: "https://posteo.de:8443/" },
	{
		id: "synology",
		name: "Synology",
		address: "https://<nas>:5001/caldav/",
	},
	{ id: "radicale", name: "Radicale", address: "http://localhost:5232/" },
	{ id: "other", name: "", address: "" },
];

export function caldavPreset(id: CalDavPresetId): CalDavPreset {
	return CALDAV_PRESETS.find((preset) => preset.id === id) as CalDavPreset;
}

/** A CalDAV address the owner finished typing: http(s), a host, no `<placeholder>` left. */
export function normalizeCalDavAddress(input: string): string {
	const trimmed = input.trim();
	if (!trimmed || /[<>]/.test(trimmed)) return "";
	const address = /^[a-z][a-z0-9+.-]*:\/\//i.test(trimmed)
		? trimmed
		: `https://${trimmed}`;
	try {
		const url = new URL(address);
		if (url.protocol !== "https:" && url.protocol !== "http:") return "";
		if (url.username || url.password || !url.hostname) return "";
		return address;
	} catch {
		return "";
	}
}

export interface CalDavForm {
	label: string;
	address: string;
	username: string;
	password: string;
}

export function calDavFormReady(form: CalDavForm): boolean {
	return (
		form.label.trim() !== "" &&
		form.username.trim() !== "" &&
		form.password !== "" &&
		normalizeCalDavAddress(form.address) !== ""
	);
}

/** The create request of a CalDAV account; the password travels as the grant only. */
export function calDavCreateRequest(
	form: CalDavForm,
): ICreateBotConnectorRequest {
	return {
		kind: "caldav",
		label: form.label.trim(),
		url: normalizeCalDavAddress(form.address),
		auth: { type: "basic", username: form.username.trim() },
		grant: form.password,
	};
}

/** The calendar the bot uses: the stored choice, else the first writable one. */
export function chosenCalendar(
	connector: Pick<IBotConnector, "calendar_url" | "calendars">,
): IBotConnectorCalendar | undefined {
	const calendars = connector.calendars ?? [];
	return (
		calendars.find((calendar) => calendar.url === connector.calendar_url) ??
		calendars.find((calendar) => calendar.writable)
	);
}

/** The discovery error means the server refused the sign-in. */
export function isCalDavSignInError(message: string): boolean {
	return /\b401\b|unauthori[sz]ed|password|credentials/i.test(message);
}

/** Accounts whose primary calendar the bot reads through an OAuth connector. */
export const CALENDAR_ACCOUNT_PROVIDERS = ["google", "microsoft"] as const;
export type CalendarAccountProvider =
	(typeof CALENDAR_ACCOUNT_PROVIDERS)[number];

/** What the calendar tools need: reading and, after asking, changing events. */
export const CALENDAR_ACCOUNT_SCOPES: Readonly<
	Record<CalendarAccountProvider, readonly string[]>
> = {
	google: ["https://www.googleapis.com/auth/calendar.events"],
	microsoft: ["Calendars.ReadWrite"],
};

const CALENDAR_SCOPE_EQUIVALENTS: Readonly<
	Record<CalendarAccountProvider, readonly string[]>
> = {
	google: [
		"https://www.googleapis.com/auth/calendar",
		"https://www.googleapis.com/auth/calendar.events",
	],
	microsoft: [
		"Calendars.ReadWrite",
		"https://graph.microsoft.com/Calendars.ReadWrite",
	],
};

/** The provider whose calendar an account connector reads, as the host matches it. */
export function calendarAccountProvider(
	connector: Pick<IBotConnector, "kind" | "provider_id">,
): CalendarAccountProvider | null {
	if (connector.kind !== "oauth") return null;
	const id = connector.provider_id?.trim().toLowerCase();
	return CALENDAR_ACCOUNT_PROVIDERS.find((provider) => provider === id) ?? null;
}

export function isCalendarConnector(
	connector: Pick<IBotConnector, "kind" | "provider_id">,
): boolean {
	return (
		connector.kind === "caldav" || calendarAccountProvider(connector) !== null
	);
}

export function hasCalendarScope(
	provider: CalendarAccountProvider,
	scopes: readonly string[],
): boolean {
	return scopes.some((scope) =>
		CALENDAR_SCOPE_EQUIVALENTS[provider].includes(scope),
	);
}

/** `scopes` plus the calendar scopes, without duplicates. */
export function withCalendarScopes(
	provider: CalendarAccountProvider,
	scopes: readonly string[],
): string[] {
	return [...new Set([...scopes, ...CALENDAR_ACCOUNT_SCOPES[provider]])];
}

/** The connector reading an account's calendar, asking for calendar access with its other scopes. */
export function calendarAccountRequest(
	provider: CalendarAccountProvider,
	label: string,
	account: Pick<IConnectedAccount, "handle" | "scopes">,
): ICreateBotConnectorRequest {
	return {
		kind: "oauth",
		label,
		auth: { type: "oauth", provider_id: provider },
		grant: account.handle,
		scopes: withCalendarScopes(provider, account.scopes),
	};
}
