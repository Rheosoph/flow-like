// Sink token management types for admin endpoints

export interface ISinkTokenInfo {
	jti: string;
	sink_type: string;
	name: string | null;
	revoked: boolean;
	revoked_at: string | null;
	revoked_by: string | null;
	created_at: string;
}

export interface IListTokensResponse {
	tokens: ISinkTokenInfo[];
	total: number;
}

export interface IListTokensQuery {
	sink_type?: string;
	include_revoked?: boolean;
}

export interface IRegisterSinkRequest {
	sink_type: string;
	name?: string;
}

export interface IRegisterSinkResponse {
	token: string;
	jti: string;
	sink_type: string;
}

export interface IRevokeSinkResponse {
	success: boolean;
	jti: string;
	message: string;
}

export const SINK_TYPES = [
	"cron",
	"discord",
	"telegram",
	"github",
	"rss",
	"mqtt",
	"email",
	"inbound_email",
	"http",
] as const;

export type ServiceSinkType = (typeof SINK_TYPES)[number];

export const SINK_TYPE_LABELS: Record<ServiceSinkType, string> = {
	cron: "Cron Jobs",
	discord: "Discord",
	telegram: "Telegram",
	github: "GitHub",
	rss: "RSS Feeds",
	mqtt: "MQTT",
	email: "Email",
	inbound_email: "Server Email",
	http: "HTTP Webhooks",
};

export const SINK_TYPE_DESCRIPTIONS: Record<ServiceSinkType, string> = {
	cron: "Scheduled task triggers (EventBridge, cron jobs)",
	discord: "Discord bot integrations",
	telegram: "Telegram bot webhooks",
	github: "GitHub webhooks and actions",
	rss: "RSS/Atom feed polling",
	mqtt: "MQTT message broker events",
	email: "IMAP mailbox polling",
	inbound_email: "Receive custom email addresses through SES or Postfix",
	http: "Generic HTTP webhooks",
};
