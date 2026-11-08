import type { ConversationRoom } from "../../hooks/use-conversation-room";

/** The y-webrtc room every member of an account conversation shares. */
export function conversationRealtimeRoom(conversationId: string): string {
	return `inbox:${conversationId}:room-v1`;
}

/** Calls with bots on this device need microphone state, but have no remote peers. */
export function createLocalConversationRoom(sub: string): ConversationRoom {
	let state: Record<string, unknown> = { sub };
	const listeners = new Set<() => void>();
	return {
		awareness: {
			clientID: 1,
			getStates: () => new Map([[1, state]]),
			getLocalState: () => state,
			setLocalStateField(field, value) {
				state = { ...state, [field]: value };
				for (const listener of listeners) listener();
			},
			on: (_event, listener) => {
				listeners.add(listener);
			},
			off: (_event, listener) => {
				listeners.delete(listener);
			},
		},
		provider: {
			room: { peerId: "local-peer", webrtcConns: new Map() },
			on() {},
			off() {},
		},
	};
}
