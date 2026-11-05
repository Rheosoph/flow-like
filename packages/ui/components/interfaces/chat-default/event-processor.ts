import { handleElementsRequestMessage } from "../../a2ui/elements-request-handler";
import { handleWidgetQueryMessage } from "../../a2ui/widget-query-handler";
import { processChatEvents as foldChatEvents } from "./chat-event-fold";

export {
	finalizePlanSteps,
	mergeChatWidgets,
	type ProcessChatEventsResult,
} from "./chat-event-fold";

/** Tab projection also serves requests for widgets mounted in this tab. */
export function processChatEvents(
	events: Parameters<typeof foldChatEvents>[0],
	state: Parameters<typeof foldChatEvents>[1],
): ReturnType<typeof foldChatEvents> {
	if (state.handleRequests !== false) {
		for (const event of events) {
			if (event.event_type !== "a2ui") continue;
			if (handleWidgetQueryMessage(event.payload)) continue;
			handleElementsRequestMessage(event.payload, () => null);
		}
	}
	return foldChatEvents(events, state);
}
