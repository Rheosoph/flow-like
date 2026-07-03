import type { AIProvider } from "@flow-like/flow-like-ui/components/flowpilot/types";
import { createId } from "@paralleldrive/cuid2";
import { create } from "zustand";
import type { IMessage } from "./global-chat-db";

export type GlobalChatMode = "closed" | "overlay";

export interface InlineAppChat {
	id: string;
	appId: string;
	eventId: string;
	/** Display name of the app / chat event for the card header. */
	name: string;
}

export interface GlobalChatDraft {
	prompt: string;
	/** Backend-prefixed model id (e.g. "github-copilot:…", "codex:…") or a raw Bits id. */
	modelId?: string;
	/** Raw browser files captured on the landing bar, forwarded to the first /chat send. */
	files?: File[];
}

/**
 * Shared state for the global FlowPilot assistant. Lives outside the surface components so the same
 * conversation renders in the full `/chat` view and in the docked bottom-right overlay: when the agent
 * navigates the user away from `/chat`, the chat morphs into the overlay without losing context.
 */
interface GlobalChatState {
	/** Pending message handed off from the landing bar to the /chat view. */
	draft: GlobalChatDraft | null;
	/** Docked overlay visibility, toggled when the agent navigates or the user opens the dock. */
	mode: GlobalChatMode;
	/** Conversation currently shown in both /chat and the overlay. */
	activeConversationId: string;
	/** Committed messages of the active conversation (streaming bubbles stay in the surface). */
	messages: IMessage[];
	isStreaming: boolean;
	provider: AIProvider;
	/** Raw (un-prefixed) model id selected in the picker. */
	selectedModelId: string;
	/** Embedding bit id used for profile-scoped memory ("" = memory off). */
	embeddingModelId: string;
	/** App chat events the agent surfaced inline in the global chat view. */
	inlineAppChats: InlineAppChat[];

	setDraft: (draft: GlobalChatDraft) => void;
	/** Returns the pending draft once and clears it, so it is only auto-sent a single time. */
	consumeDraft: () => GlobalChatDraft | null;
	openOverlay: () => void;
	closeOverlay: () => void;
	appendMessage: (message: IMessage) => void;
	setStreaming: (streaming: boolean) => void;
	setProvider: (provider: AIProvider) => void;
	setSelectedModelId: (modelId: string) => void;
	setEmbeddingModelId: (modelId: string) => void;
	addInlineAppChat: (chat: Omit<InlineAppChat, "id">) => void;
	removeInlineAppChat: (id: string) => void;
	/** Start a fresh conversation (new id, cleared transcript). */
	newConversation: () => void;
	/** Resume a persisted conversation from history. */
	loadConversation: (conversationId: string, messages: IMessage[]) => void;
}

export const useGlobalChatStore = create<GlobalChatState>((set, get) => ({
	draft: null,
	mode: "closed",
	activeConversationId: createId(),
	messages: [],
	isStreaming: false,
	provider: "bits",
	selectedModelId: "",
	embeddingModelId: "",
	inlineAppChats: [],

	setDraft: (draft) => set({ draft }),
	consumeDraft: () => {
		const { draft } = get();
		if (draft) set({ draft: null });
		return draft;
	},
	openOverlay: () => set({ mode: "overlay" }),
	closeOverlay: () => set({ mode: "closed" }),
	appendMessage: (message) =>
		set((state) => ({ messages: [...state.messages, message] })),
	setStreaming: (isStreaming) => set({ isStreaming }),
	setProvider: (provider) => set({ provider }),
	setSelectedModelId: (selectedModelId) => set({ selectedModelId }),
	setEmbeddingModelId: (embeddingModelId) => set({ embeddingModelId }),
	addInlineAppChat: (chat) =>
		set((state) => {
			// One card per (app, event) — surfacing the same chat twice just keeps the existing card.
			if (
				state.inlineAppChats.some(
					(existing) =>
						existing.appId === chat.appId && existing.eventId === chat.eventId,
				)
			) {
				return state;
			}
			return {
				inlineAppChats: [...state.inlineAppChats, { ...chat, id: createId() }],
			};
		}),
	removeInlineAppChat: (id) =>
		set((state) => ({
			inlineAppChats: state.inlineAppChats.filter((chat) => chat.id !== id),
		})),
	newConversation: () =>
		set({
			activeConversationId: createId(),
			messages: [],
			isStreaming: false,
			mode: "closed",
			inlineAppChats: [],
		}),
	loadConversation: (conversationId, messages) =>
		set({
			activeConversationId: conversationId,
			messages,
			isStreaming: false,
			inlineAppChats: [],
		}),
}));
