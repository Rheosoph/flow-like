import { ChatInterface } from "../components/interfaces/chat-default";
import { CronJobConfig } from "../components/interfaces/configs/cron";
import { GeolocationConfig } from "../components/interfaces/configs/geolocation";
import { DaemonConfig } from "../components/interfaces/configs/daemon";
import { DeeplinkConfig } from "../components/interfaces/configs/deeplink";
import { DiscordConfig } from "../components/interfaces/configs/discord";
import { GenericFormConfig } from "../components/interfaces/configs/generic_form";
import { HttpConfig } from "../components/interfaces/configs/http";
import { McpConfig } from "../components/interfaces/configs/mcp";
import { RestConfig } from "../components/interfaces/configs/rest";
import { SimpleChatConfig } from "../components/interfaces/configs/simple_chat";
import { TelegramConfig } from "../components/interfaces/configs/telegram";
import { UserMailConfig } from "../components/interfaces/configs/user_mail";
import { GenericEventFormInterface } from "../components/interfaces/generic-event-form";
import type { IEventMapping } from "../components/interfaces/interfaces";
import { EVENT_DEFINITIONS } from "./event-definitions";

export {
	EVENT_DEFINITIONS,
	isChatEventType,
} from "./event-definitions";

export const EVENT_CONFIG: IEventMapping = {
	events_location: {
		...EVENT_DEFINITIONS.events_location,
		configInterfaces: { geolocation: GeolocationConfig },
		useInterfaces: {},
	},
	events_chat: {
		...EVENT_DEFINITIONS.events_chat,
		configInterfaces: {
			simple_chat: SimpleChatConfig,
			discord: DiscordConfig,
			telegram: TelegramConfig,
		},
		useInterfaces: {
			simple_chat: ChatInterface,
		},
	},
	events_mail: {
		...EVENT_DEFINITIONS.events_mail,
		configInterfaces: {
			// Keyed by event type: eventTypes is ["email"], so a `user_mail` key
			// resolves to nothing and the mail config never renders.
			email: UserMailConfig,
			user_mail: UserMailConfig,
		},
		useInterfaces: {},
	},
	events_generic: {
		...EVENT_DEFINITIONS.events_generic,
		configInterfaces: {
			generic_form: GenericFormConfig,
			api: HttpConfig,
			deeplink: DeeplinkConfig,
		},
		useInterfaces: {
			generic_form: GenericEventFormInterface,
		},
	},
	events_simple: {
		...EVENT_DEFINITIONS.events_simple,
		configInterfaces: {
			quick_action: GenericFormConfig,
			api: HttpConfig,
			cron: CronJobConfig,
			daemon: DaemonConfig,
			deeplink: DeeplinkConfig,
			rest: RestConfig,
			mcp: McpConfig,
		},
		useInterfaces: {
			quick_action: GenericEventFormInterface,
		},
	},
};
