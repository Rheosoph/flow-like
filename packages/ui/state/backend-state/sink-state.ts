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
	personal_access_token?: string;
	oauth_tokens?: Record<string, unknown>;
}

export interface ISinkState {
	listEventSinks(): Promise<IEventRegistration[]>;
	removeEventSink(eventId: string): Promise<void>;
	isEventSinkActive(eventId: string): Promise<boolean>;
}
