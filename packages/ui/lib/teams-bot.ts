import { ApiResponseError } from "./api-error";

export const TEAMS_AUTH_MODES = [
	"flow_like_managed",
	"customer_teams",
	"customer_azure",
] as const;

export type TeamsAuthMode = (typeof TEAMS_AUTH_MODES)[number];

export type TeamsConnectionStatus =
	| "not_configured"
	| "provisioning"
	| "ready"
	| "setup_failed"
	| "disconnecting"
	| "disconnected"
	| "disconnect_failed";

export interface TeamsBotConnection {
	managed_available: boolean;
	configured: boolean;
	status: string;
	endpoint: string;
	connection_id: string;
	mode: TeamsAuthMode | null;
	name: string;
	description: string;
	customer_tenant_id: string;
	home_tenant_id: string;
	client_id: string;
	secret_expires_at: string | null;
	allowed_responders: string[];
	last_activity_at?: number | null;
}

export interface TeamsBotSetup {
	mode: TeamsAuthMode;
	name: string;
	description: string;
	customer_tenant_id: string;
	home_tenant_id: string;
	client_id: string;
	client_secret: string;
	allowed_responders: string[];
}

export interface TeamsBotPackage {
	filename: string;
	base64: string;
}

export type TeamsSetupField =
	| "name"
	| "description"
	| "customer_tenant_id"
	| "allowed_responders"
	| "home_tenant_id"
	| "client_id"
	| "client_secret";

const MAX_NAME = 30;
const MAX_DESCRIPTION = 4000;
const MAX_RESPONDERS = 100;

export function isMicrosoftId(value: string): boolean {
	return /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(
		value.trim(),
	);
}

export function parseTeamsApprovers(text: string): string[] {
	return text.split(/[\s,]+/).filter(Boolean);
}

/** A disconnected or never-configured bot can switch its management path. */
export function isTeamsBotLocked(saved?: TeamsBotConnection): boolean {
	return (
		!!saved?.mode && !["disconnected", "not_configured"].includes(saved.status)
	);
}

function keepsSavedSecret(input: TeamsBotSetup, saved?: TeamsBotConnection) {
	return (
		saved !== undefined &&
		saved.status !== "disconnected" &&
		saved.client_id === input.client_id.trim() &&
		saved.home_tenant_id === input.home_tenant_id.trim()
	);
}

function sharedFieldValid(field: TeamsSetupField, input: TeamsBotSetup) {
	switch (field) {
		case "name":
			return !!input.name.trim() && [...input.name].length <= MAX_NAME;
		case "description":
			return [...input.description].length <= MAX_DESCRIPTION;
		case "customer_tenant_id":
			return isMicrosoftId(input.customer_tenant_id);
		default:
			return (
				input.allowed_responders.length <= MAX_RESPONDERS &&
				input.allowed_responders.every(isMicrosoftId)
			);
	}
}

function customerFieldValid(
	field: TeamsSetupField,
	input: TeamsBotSetup,
	saved?: TeamsBotConnection,
) {
	if (field === "home_tenant_id") return isMicrosoftId(input.home_tenant_id);
	if (field === "client_id") return isMicrosoftId(input.client_id);
	return !!input.client_secret || keepsSavedSecret(input, saved);
}

const SHARED_FIELDS: readonly TeamsSetupField[] = [
	"name",
	"description",
	"customer_tenant_id",
	"allowed_responders",
];
const CUSTOMER_FIELDS: readonly TeamsSetupField[] = [
	"home_tenant_id",
	"client_id",
	"client_secret",
];

/** Every invalid field, in form order. Empty when the setup can be saved. */
export function validateTeamsSetup(
	input: TeamsBotSetup,
	saved?: TeamsBotConnection,
): TeamsSetupField[] {
	const shared = SHARED_FIELDS.filter(
		(field) => !sharedFieldValid(field, input),
	);
	if (input.mode === "flow_like_managed") return shared;
	return [
		...shared,
		...CUSTOMER_FIELDS.filter(
			(field) => !customerFieldValid(field, input, saved),
		),
	];
}

/**
 * Whether saving would change who the connected bot is or who can reach it,
 * as opposed to a metadata-only edit (name, description, approvers).
 */
export function teamsSetupChangesConnectedBot(
	input: TeamsBotSetup,
	saved?: TeamsBotConnection,
): boolean {
	if (saved?.status !== "ready") return false;
	if (input.mode !== saved.mode) return true;
	if (input.customer_tenant_id.trim() !== saved.customer_tenant_id) return true;
	if (input.mode === "flow_like_managed") return false;
	return (
		!!input.client_secret ||
		input.client_id.trim() !== saved.client_id ||
		input.home_tenant_id.trim() !== saved.home_tenant_id
	);
}

/** Switching to the managed path drops customer credentials from the draft. */
export function withTeamsAuthMode(
	setup: TeamsBotSetup,
	mode: TeamsAuthMode,
): TeamsBotSetup {
	if (mode !== "flow_like_managed") return { ...setup, mode };
	return {
		...setup,
		mode,
		home_tenant_id: "",
		client_id: "",
		client_secret: "",
	};
}

export function teamsSetupFromConnection(c: TeamsBotConnection): TeamsBotSetup {
	return {
		mode:
			c.mode ?? (c.managed_available ? "flow_like_managed" : "customer_teams"),
		name: c.name,
		description: c.description,
		customer_tenant_id: c.customer_tenant_id,
		home_tenant_id: c.home_tenant_id,
		client_id: c.client_id,
		client_secret: "",
		allowed_responders: c.allowed_responders,
	};
}

export function teamsPackageFilename(result: TeamsBotPackage): string {
	const name = result.filename?.trim();
	if (!name) return "teams-app.zip";
	return /\.zip$/i.test(name) ? name : `${name}.zip`;
}

export type TeamsOperation = "setup" | "download" | "rotate" | "disconnect";

export type TeamsFailure =
	| "forbidden"
	| "busy"
	| "managed_limit"
	| "rate_limited"
	| "managed_rejected"
	| "rejected"
	| "unavailable"
	| "unknown";

const FAILURE_BY_STATUS: Readonly<Record<number, TeamsFailure>> = {
	403: "forbidden",
	409: "busy",
	422: "managed_limit",
	429: "rate_limited",
};

/** Classifies an API failure so the UI can say what to do next. */
export function teamsFailure(
	error: unknown,
	operation: TeamsOperation,
	mode?: TeamsAuthMode,
): TeamsFailure {
	if (!(error instanceof ApiResponseError)) return "unknown";
	if (error.status >= 500) return "unavailable";
	if (error.status === 400 && operation === "setup")
		return mode === "flow_like_managed" ? "managed_rejected" : "rejected";
	return FAILURE_BY_STATUS[error.status] ?? "unknown";
}

/** The server's own explanation, without the `[CODE]` prefix. */
export function teamsFailureDetail(error: unknown): string {
	if (error instanceof ApiResponseError && error.serverMessage.trim())
		return error.serverMessage;
	return error instanceof Error ? error.message : String(error);
}
