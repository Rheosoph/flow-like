import { ChatInterface } from "../components/interfaces/chat-default";
import { GenericEventFormInterface } from "../components/interfaces/generic-event-form";
import type { IUseEventMapping } from "../components/interfaces/interfaces";

/**
 * The runtime half of `EVENT_CONFIG`: which event types a running app can render, and with
 * what.
 *
 * `EVENT_CONFIG` also carries every event *configuration* panel, and reaching it pulls the
 * whole builder — the flow editor, the code editors, the chart libraries — into whatever
 * imports it. `/use` renders none of that, so it imports this instead, by module path rather
 * than through the package barrel. Both must list the same event types: a type present here
 * without an interface simply never renders.
 */
export const USE_EVENT_CONFIG: IUseEventMapping = {
	events_chat: {
		eventTypes: ["simple_chat", "discord", "telegram"],
		useInterfaces: {
			simple_chat: ChatInterface,
		},
	},
	events_mail: {
		eventTypes: ["email"],
		useInterfaces: {},
	},
	events_generic: {
		eventTypes: ["generic_form", "api", "deeplink"],
		useInterfaces: {
			generic_form: GenericEventFormInterface,
		},
	},
	events_simple: {
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
	},
};
