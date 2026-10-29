/**
 * Sink state interface for managing active event sinks (desktop only)
 */

export type SinkType =
	| "discord"
	| "email"
	| "slack"
	| "telegram"
	| "web_watcher"
	| "rss"
	| "deeplink"
	| "http"
	| "webhook"
	| "mqtt"
	| "mcp"
	| "file"
	| "github"
	| "nfc"
	| "geolocation"
	| "notion"
	| "shortcut"
	| "cron"
	| "daemon";

/**
 * Whether a trigger runs on this computer: `none` when it runs elsewhere, `unregistered` when it
 * should run here but has no registration, `needs_token` when it has no personal access token of
 * this account to sign in to the hub with, `failed` when it last failed to start or refresh.
 */
export type LocalTriggerState =
	| "none"
	| "unregistered"
	| "ready"
	| "needs_token"
	| { failed: { reason: string } };

/** A trigger this computer registered. Its personal access token and OAuth grants never leave the desktop. */
export interface IEventRegistration {
	event_id: string;
	name: string;
	type: SinkType | string;
	updated_at: number;
	created_at: number;
	config: Record<string, unknown>;
	offline: boolean;
	app_id: string;
	default_payload?: unknown;
	has_token?: boolean;
	/** The account the stored token belongs to; `null` on a registration from before it was recorded. */
	pat_owner_sub?: string | null;
	state?: LocalTriggerState;
	/** Providers the registration holds OAuth grants for. */
	oauth_providers?: string[];
}

export interface ISinkState {
	listEventSinks(): Promise<IEventRegistration[]>;
	removeEventSink(eventId: string): Promise<void>;
	isEventSinkActive(eventId: string): Promise<boolean>;
}
