import { create } from "zustand";

interface CompactionRequestState {
	/** Chats whose next reply first folds the older conversation into a summary. */
	readonly requested: Readonly<Record<string, true>>;
	setRequested: (conversationId: string, requested: boolean) => void;
}

export const useCompactionRequestStore = create<CompactionRequestState>(
	(set) => ({
		requested: {},
		setRequested: (conversationId, requested) =>
			set((state) => {
				if (Boolean(state.requested[conversationId]) === requested)
					return state;
				const next = { ...state.requested };
				if (requested) next[conversationId] = true;
				else delete next[conversationId];
				return { requested: next };
			}),
	}),
);

/** Whether the chat's next reply compacts first. The request ends here: that reply carries it. */
export function takeCompactionRequest(conversationId: string): boolean {
	const store = useCompactionRequestStore.getState();
	if (!store.requested[conversationId]) return false;
	store.setRequested(conversationId, false);
	return true;
}
