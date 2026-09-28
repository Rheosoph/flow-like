import {
	ChatInterface,
	CronJobConfig,
	DaemonConfig,
	DeeplinkConfig,
	DiscordConfig,
	GenericEventFormInterface,
	GenericFormConfig,
	HttpConfig,
	type IEventMapping,
	McpConfig,
	RestConfig,
	SimpleChatConfig,
	TelegramConfig,
	UserMailConfig,
} from "@flow-like/flow-like-ui";
import { InboundEmailConfig } from "@flow-like/flow-like-ui/components/interfaces/configs/inbound-email";
import { TeamsConfig } from "@flow-like/flow-like-ui/components/interfaces/configs/teams";
import { EVENT_DEFINITIONS } from "@flow-like/flow-like-ui/lib/event-definitions";
import { createId } from "@paralleldrive/cuid2";

export const EVENT_CONFIG: IEventMapping = {
	events_inbound_email: {
		...EVENT_DEFINITIONS.events_inbound_email,
		configInterfaces: { inbound_email: InboundEmailConfig },
		useInterfaces: {},
	},
	events_chat: {
		...EVENT_DEFINITIONS.events_chat,
		configInterfaces: {
			simple_chat: SimpleChatConfig,
			discord: DiscordConfig,
			telegram: TelegramConfig,
			teams: TeamsConfig,
		},
		useInterfaces: { simple_chat: ChatInterface },
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
			api: HttpConfig,
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
			api: HttpConfig,
			cron: CronJobConfig,
			daemon: DaemonConfig,
			deeplink: DeeplinkConfig,
			rest: RestConfig,
			mcp: McpConfig,
		},
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
		useInterfaces: {
			quick_action: GenericEventFormInterface,
		},
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
