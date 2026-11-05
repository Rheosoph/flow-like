import { ConversationFeedHub } from "@flow-like/flow-like-ui/lib/realtime/conversation-feed-hub";

const changeListeners = new Set<() => void>();

/** Watchers of open conversations; the native realtime bridge serves account ones, the local bridge local ones. */
export const desktopConversationFeed = new ConversationFeedHub(() => {
	for (const listener of [...changeListeners]) listener();
});

export function onWatchedConversationsChange(listener: () => void): () => void {
	changeListeners.add(listener);
	return () => {
		changeListeners.delete(listener);
	};
}
