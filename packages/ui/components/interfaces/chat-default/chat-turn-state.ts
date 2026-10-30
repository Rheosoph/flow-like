import { Response } from "../../../lib/llm/response";
import type { IIntercomEvent } from "../../../lib/schema/events/intercom-event";
import type {
	IAttachment,
	IGlobalState,
	ILocalChatState,
	IMessage,
} from "./chat-db";
import {
	type ProcessChatEventsResult,
	processChatEvents,
} from "./chat-event-fold";

/** What a live subscriber folded so far; complete once it consumed every accumulated event. */
export interface IStreamLiveState {
	responseMessages: IMessage[];
	tmpLocalState: ILocalChatState | null | undefined;
	tmpGlobalState: IGlobalState | null | undefined;
	processedEvents: number;
}

/** One assistant turn folded from its run's event stream, batch by batch. */
export interface IChatTurn {
	readonly responseMessage: IMessage;
	readonly responseMessages: IMessage[];
	/** Folds the batch that follows everything folded so far. */
	fold(events: IIntercomEvent[]): ProcessChatEventsResult;
	/**
	 * Folds the part of the run's full event list that nothing folded yet. While a subscriber is
	 * attached that is nothing; after it detached (remount, session switch) the saver catches the
	 * turn up, so its checkpoint is the whole answer rather than the moment the subscriber left.
	 */
	catchUp(events: readonly IIntercomEvent[]): void;
	live(): IStreamLiveState;
}

export function createChatTurn(
	options: {
		responseMessage: IMessage;
		tmpLocalState: ILocalChatState | null | undefined;
		tmpGlobalState: IGlobalState | null | undefined;
		appId: string;
		eventId: string;
		sessionId: string;
		handleRequests?: boolean;
	},
	foldEvents = processChatEvents,
): IChatTurn {
	const { appId, eventId, sessionId } = options;
	let responseMessage = options.responseMessage;
	let responseMessages = [responseMessage];
	let { tmpLocalState, tmpGlobalState } = options;
	let intermediateResponse = Response.default();
	let attachments = new Map<string, IAttachment>();
	let done = false;
	let processedEvents = 0;

	const fold = (events: IIntercomEvent[]) => {
		const result = foldEvents(events, {
			intermediateResponse,
			responseMessage,
			responseMessages,
			attachments,
			tmpLocalState,
			tmpGlobalState,
			done,
			appId,
			eventId,
			sessionId,
			handleRequests: options.handleRequests,
		});
		processedEvents += events.length;
		intermediateResponse = result.intermediateResponse;
		responseMessage = result.responseMessage;
		responseMessages = result.responseMessages;
		attachments = result.attachments;
		tmpLocalState = result.tmpLocalState;
		tmpGlobalState = result.tmpGlobalState;
		done = result.done;
		return result;
	};

	return {
		get responseMessage() {
			return responseMessage;
		},
		get responseMessages() {
			return responseMessages;
		},
		fold,
		catchUp(events) {
			if (events.length > processedEvents) fold(events.slice(processedEvents));
		},
		live: () => ({
			responseMessages,
			tmpLocalState,
			tmpGlobalState,
			processedEvents,
		}),
	};
}
