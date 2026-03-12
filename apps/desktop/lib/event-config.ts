import { createId } from "@paralleldrive/cuid2";
import {
	ApiConfig,
	ChatInterface,
	CronJobConfig,
	DeeplinkConfig,
	DiscordConfig,
	GenericEventFormInterface,
	GenericFormConfig,
	type IEventMapping,
	SimpleChatConfig,
	TelegramConfig,
	UserMailConfig,
} from "@tm9657/flow-like-ui";

export const EVENT_CONFIG: IEventMapping = {
	events_chat: {
		configInterfaces: {
			simple_chat: SimpleChatConfig,
			discord: DiscordConfig,
			telegram: TelegramConfig,
		},
		useInterfaces: {
			simple_chat: ChatInterface,
		},
		configs: {
			simple_chat: {
				allow_file_upload: true,
				allow_voice_input: false,
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
		eventTypes: ["simple_chat", "advanced_chat", "discord", "telegram"],
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
		configInterfaces: {
			user_mail: UserMailConfig,
		},
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
		useInterfaces: {},
		withSink: ["email"],
		sinkAvailability: {
			email: {
				availability: "local",
				description: "Requires IMAP connection (desktop only)",
			},
		},
	},
	events_generic: {
		configInterfaces: {
			generic_form: GenericFormConfig,
			api: ApiConfig,
			deeplink: DeeplinkConfig,
		},
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
		useInterfaces: {
			generic_form: GenericEventFormInterface,
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
		configInterfaces: {
			quick_action: GenericFormConfig,
			api: ApiConfig,
			cron: CronJobConfig,
			deeplink: DeeplinkConfig,
		},
		defaultEventType: "quick_action",
		eventTypes: ["quick_action", "api", "cron", "deeplink"],
		useInterfaces: {
			quick_action: GenericEventFormInterface,
		},
		withSink: ["cron", "api", "deeplink"],
		sinkAvailability: {
			cron: {
				availability: "both",
				description: "Scheduled execution - runs locally or on server",
			},
			api: {
				availability: "both",
				description: "HTTP endpoint - runs locally or on server",
			},
			deeplink: {
				availability: "local",
				description: "Deep links only work on desktop",
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
				expression: "* */1 * * *",
			},
			deeplink: {
				sink_type: "deeplink",
				route: createId(),
			},
		},
	},
};
