import { useMemo } from "react";
import type { ConversationRoom } from "../../packages/ui/hooks/use-conversation-room";

export function useMeetingRoom({ participantId }: { participantId: string }) {
	const room = useMemo<ConversationRoom>(() => {
		const listeners = new Set<() => void>();
		const local: Record<string, unknown> = { sub: participantId };
		return {
			awareness: {
				clientID: 1,
				getLocalState: () => local,
				getStates: () => new Map([[1, local]]),
				setLocalStateField(field, value) {
					local[field] = value;
					for (const listener of listeners) listener();
				},
				on(_event, listener) {
					listeners.add(listener);
				},
				off(_event, listener) {
					listeners.delete(listener);
				},
			},
			provider: {
				room: { peerId: participantId, webrtcConns: new Map() },
				on() {},
				off() {},
			},
		};
	}, [participantId]);
	return { room, error: undefined };
}
