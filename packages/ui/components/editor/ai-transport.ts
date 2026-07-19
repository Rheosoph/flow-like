import type { IHistoryMessage } from "../../lib";
import type { IAIState } from "../../state/backend-state/ai-state";

/** Keep both editor AI transports on the same app-attribution contract. */
export const streamEditorChat = (
	aiState: IAIState,
	messages: IHistoryMessage[],
	appId?: string,
) => aiState.streamChatComplete(messages, appId);

export const completeEditorChat = (
	aiState: IAIState,
	messages: IHistoryMessage[],
	appId?: string,
) => aiState.chatComplete(messages, appId);
