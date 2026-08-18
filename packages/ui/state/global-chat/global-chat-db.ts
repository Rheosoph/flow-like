import Dexie, { type EntityTable } from "dexie";
import type {
	IChatRunContext,
	IMessage,
	ISession,
} from "../../components/interfaces/chat-default/chat-db";

export type { IChatRunContext, IMessage, ISession };

/** Synthetic appId used for the global platform assistant (not tied to a single Flow-Like app). */
export const GLOBAL_CHAT_APP_ID = "global";

/** Global-chat session row: the shared chat session plus optional surface scoping. */
export interface IGlobalChatSession extends ISession {
	/** Board this conversation was scoped to, when started from an open flow board. */
	boardId?: string;
	/** Conversation scope: platform-wide ("global"), board copilot ("board"), or UI builder ("ui"). */
	mode?: "global" | "board" | "ui";
	/**
	 * Epoch millis the user pinned this conversation; absent means unpinned. A timestamp rather
	 * than a boolean because booleans are not valid IndexedDB keys — `pinned` would index nothing —
	 * and it gives pinned rows a stable order for free.
	 */
	pinnedAt?: number;
}

/**
 * Frontend-only history for the global FlowPilot assistant. Mirrors the chat-default schema but is a
 * separate Dexie database so global conversations never mix with per-app chat history. Cloud sync is
 * a planned fast-follow; for now history is per-device.
 */
const globalChatDb = new Dexie("Global-Chat-History") as Dexie & {
	sessions: EntityTable<IGlobalChatSession, "id">;
	messages: EntityTable<IMessage, "id">;
};

globalChatDb.version(1).stores({
	sessions: "id, updatedAt",
	messages: "id, sessionId, timestamp",
});

globalChatDb.version(2).stores({
	sessions: "id, updatedAt, boardId, mode",
	messages: "id, sessionId, timestamp",
});

globalChatDb.version(3).stores({
	sessions: "id, updatedAt, boardId, mode, pinnedAt",
	messages: "id, sessionId, timestamp",
});

export { globalChatDb };
