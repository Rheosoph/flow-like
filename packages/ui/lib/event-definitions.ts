import { createId } from "@paralleldrive/cuid2";
import { DEFAULT_CHAT_AI_DISCLOSURE } from "./chat-appearance";
import { DEFAULT_CHAT_THEME_CSS } from "./chat-theme-presets";
import type { IEventPayload } from "./schema/flow/event-payload";

export type EventSinkAvailability = "local" | "remote" | "both";

export interface EventSinkDefinition {
	availability: EventSinkAvailability;
	description?: string;
	/** The trigger stays on this device but can invoke a Remote Event. */
	dispatchesRemoteEvents?: boolean;
}

export function sinkSupportsEventExecution(
	sink: EventSinkDefinition | null | undefined,
	mode?: "Local" | "Remote",
	canUseLocalSink = false,
): boolean {
	if (sink?.availability === "local" && sink.dispatchesRemoteEvents)
		return canUseLocalSink;
	if (!sink || !mode || sink.availability === "both") return true;
	return mode === "Local"
		? sink.availability === "local"
		: sink.availability === "remote";
}

export interface EventDefinition {
	configs: Record<string, Partial<IEventPayload>>;
	eventTypes: string[];
	defaultEventType: string;
	withSink: string[];
	sinkAvailability?: Record<string, EventSinkDefinition>;
}

export type EventDefinitionMapping = Record<string, EventDefinition>;

/** Whether an Event renders the built-in chat interface. */
export function isChatEventType(eventType: string): boolean {
	return eventType === "simple_chat";
}

/**
 * Event types and their persisted defaults. This module deliberately has no UI component
 * imports so host tools, validators, and tests can use the catalog without loading React.
 */
export const EVENT_DEFINITIONS: EventDefinitionMapping = {
	events_location: {
		defaultEventType: "geolocation",
		eventTypes: ["geolocation"],
		withSink: ["geolocation"],
		configs: {
			geolocation: {
				sink_type: "geolocation",
				latitude: 0,
				longitude: 0,
				radius: 100,
				trigger_on: "Both",
				background: false,
			},
		},
		sinkAvailability: {
			geolocation: {
				availability: "local",
				dispatchesRemoteEvents: true,
				description:
					"This device monitors the region and invokes the Event locally or on its configured server.",
			},
		},
	},
	events_chat: {
		configs: {
			simple_chat: {
				allow_file_upload: true,
				allow_voice_input: false,
				ai_disclosure: DEFAULT_CHAT_AI_DISCLOSURE,
				background_image: "",
				custom_css: DEFAULT_CHAT_THEME_CSS,
				voice: {
					mode: "disabled",
					invoke: "manual",
					variant: "conservative",
					size: "md",
					playback: "text",
					max_duration: 300,
					auto_stop: false,
				},
				history_elements: 5,
				tools: [],
				default_tools: [],
				example_messages: [],
			},
			discord: {
				sink_type: "discord",
				token: "",
				bot_name: "Flow-Like Bot",
				bot_description: "",
				intents: ["Guilds", "GuildMessages", "MessageContent"],
				channel_whitelist: [],
				channel_blacklist: [],
				respond_to_mentions: true,
				respond_to_dms: true,
				command_prefix: "!",
			},
			telegram: {
				sink_type: "telegram",
				bot_token: "",
				bot_name: "Flow-Like Bot",
				bot_description: "",
				chat_whitelist: [],
				chat_blacklist: [],
				respond_to_mentions: true,
				respond_to_private: true,
				command_prefix: "/",
			},
		},
		defaultEventType: "simple_chat",
		eventTypes: ["simple_chat", "discord", "telegram"],
		withSink: ["discord", "telegram"],
		sinkAvailability: {
			discord: {
				availability: "local",
				description: "Requires persistent connection to Discord",
			},
			telegram: {
				availability: "local",
				description: "Requires persistent connection to Telegram",
			},
		},
	},
	events_mail: {
		defaultEventType: "email",
		eventTypes: ["email"],
		configs: {
			email: {
				sink_type: "email",
				imap_server: "",
				imap_port: 993,
				username: "",
				password: "",
				use_tls: true,
			},
		},
		withSink: ["email"],
		sinkAvailability: {
			email: {
				availability: "local",
				description: "Requires IMAP connection (desktop only)",
			},
		},
	},
	events_generic: {
		defaultEventType: "generic_form",
		eventTypes: ["generic_form", "api", "deeplink"],
		configs: {
			generic_form: {},
			api: {
				sink_type: "http",
				method: "GET",
				path: `/${createId()}`,
				public_endpoint: false,
			},
			deeplink: {
				sink_type: "deeplink",
				route: createId(),
			},
		},
		withSink: ["api", "deeplink"],
		sinkAvailability: {
			api: {
				availability: "both",
				description: "HTTP endpoint - runs locally or on server",
			},
			deeplink: {
				availability: "local",
				description: "Deep links only work on desktop",
			},
		},
	},
	events_simple: {
		defaultEventType: "quick_action",
		eventTypes: [
			"quick_action",
			"api",
			"cron",
			"daemon",
			"deeplink",
			"rest",
			"mcp",
		],
		withSink: ["cron", "api", "daemon", "deeplink", "rest", "mcp"],
		sinkAvailability: {
			cron: {
				availability: "both",
				description: "Scheduled execution - runs locally or on server",
			},
			daemon: {
				availability: "local",
				description: "Long-running supervised local workflow",
			},
			api: {
				availability: "both",
				description: "HTTP endpoint - runs locally or on server",
			},
			deeplink: {
				availability: "local",
				description: "Deep links only work on desktop",
			},
			rest: {
				availability: "remote",
				description: "Multi-endpoint REST API server with auth - remote only",
			},
			mcp: {
				availability: "remote",
				description: "Model Context Protocol server - remote only",
			},
		},
		configs: {
			api: {
				sink_type: "http",
				method: "GET",
				path: `/${createId()}`,
				public_endpoint: false,
			},
			cron: {
				sink_type: "cron",
				expression: "0 */1 * * *",
			},
			daemon: {
				sink_type: "daemon",
				restart_policy: "on_failure",
				min_restart_delay_ms: 1000,
				max_restart_delay_ms: 30000,
				board_poll_interval_ms: 3000,
				log_flush_interval_ms: 5000,
				log_batch_size: 500,
				healthy_reset_ms: 60000,
			},
			deeplink: {
				sink_type: "deeplink",
				route: createId(),
			},
			rest: {
				sink_type: "rest",
			},
			mcp: {
				sink_type: "mcp",
			},
		},
	},
};
