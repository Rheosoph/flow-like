export interface IBotDiscordSettings {
	enabled: boolean;
	respond_to_mentions: boolean;
	respond_to_dms: boolean;
	channel_allow: string[];
}

export interface IBotDiscordView extends IBotDiscordSettings {
	has_token: boolean;
	status: "stopped" | "connecting" | "connected" | "error";
	error?: string | null;
	bot_user?: { id: string; name: string } | null;
	invite_url?: string | null;
}

export interface IBotDiscordInput extends IBotDiscordSettings {
	token?: string | null;
}

export interface IBotDiscordState {
	get(botId: string): Promise<IBotDiscordView | null>;
	set(botId: string, input: IBotDiscordInput): Promise<IBotDiscordView>;
	remove(botId: string): Promise<void>;
}

export type BotChannelProvider = "teams" | "discord";

export interface IBotChannelLink {
	id: string;
	provider: BotChannelProvider;
	chat_title: string;
	conversation_id: string;
	created_at: string;
	last_seen_at: string;
}
