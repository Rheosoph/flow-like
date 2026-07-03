import type {
	IMessage,
	ISession,
} from "@flow-like/flow-like-ui/components/interfaces/chat-default/chat-db";
import Dexie, { type EntityTable } from "dexie";

export type { IMessage, ISession };

/** Synthetic appId used for the global platform assistant (not tied to a single Flow-Like app). */
export const GLOBAL_CHAT_APP_ID = "global";

/**
 * Frontend-only history for the global FlowPilot assistant. Mirrors the chat-default schema but is a
 * separate Dexie database so global conversations never mix with per-app chat history. Cloud sync is
 * a planned fast-follow; for now history is per-device.
 */
const globalChatDb = new Dexie("Global-Chat-History") as Dexie & {
	sessions: EntityTable<ISession, "id">;
	messages: EntityTable<IMessage, "id">;
};

globalChatDb.version(1).stores({
	sessions: "id, updatedAt",
	messages: "id, sessionId, timestamp",
});

export { globalChatDb };
