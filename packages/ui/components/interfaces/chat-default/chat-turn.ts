import { createChatTurn as createTurnState } from "./chat-turn-state";
import { processChatEvents } from "./event-processor";

export type { IChatTurn, IStreamLiveState } from "./chat-turn-state";

export function createChatTurn(options: Parameters<typeof createTurnState>[0]) {
	return createTurnState(options, processChatEvents);
}
