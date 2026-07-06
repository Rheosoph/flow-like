import { IRole } from "@flow-like/flow-like-ui";
import { flowpilotDB } from "@flow-like/flow-like-ui/lib/flowpilot-db";
import {
	GLOBAL_CHAT_APP_ID,
	type IGlobalChatSession,
	type IMessage,
	globalChatDb,
} from "./global-chat-db";

/** One-time flag so the legacy import runs a single time per device. */
const MIGRATION_FLAG = "flow-like:flowpilot-history-migrated";

function toMillis(iso: string | undefined): number {
	const parsed = iso ? Date.parse(iso) : Number.NaN;
	return Number.isFinite(parsed) ? parsed : Date.now();
}

/**
 * Best-effort, one-time import of the legacy board/widget FlowPilot history
 * ("FlowPilotHistory" Dexie DB) into the global chat DB, so old conversations
 * stay reachable from the unified assistant's history. The legacy DB is left
 * untouched (read-only) — only the localStorage flag marks completion.
 */
export async function migrateFlowPilotHistory(): Promise<void> {
	try {
		if (typeof window === "undefined") return;
		if (localStorage.getItem(MIGRATION_FLAG)) return;

		const conversations = await flowpilotDB.conversations.toArray();
		for (const conv of conversations) {
			try {
				const existing = await globalChatDb.sessions.get(conv.id);
				if (existing) continue;

				const messages = await flowpilotDB.messages
					.where("conversationId")
					.equals(conv.id)
					.sortBy("createdAt");

				const appId = conv.appId ?? GLOBAL_CHAT_APP_ID;
				const firstUserContent = messages.find(
					(m) => m.role === "user" && m.content,
				)?.content;
				const session: IGlobalChatSession = {
					id: conv.id,
					appId,
					summarization: (
						conv.title ||
						firstUserContent ||
						"FlowPilot conversation"
					).slice(0, 80),
					createdAt: toMillis(conv.createdAt),
					updatedAt: toMillis(conv.updatedAt),
					boardId: conv.boardId,
					mode:
						conv.mode === "board"
							? "board"
							: conv.mode === "ui"
								? "ui"
								: "global",
				};

				const mapped: IMessage[] = messages.map((msg) => ({
					id: msg.id,
					appId,
					sessionId: conv.id,
					inner: {
						role: msg.role === "assistant" ? IRole.Assistant : IRole.User,
						content: msg.content,
					},
					files: [],
					timestamp: toMillis(msg.createdAt),
				}));

				await globalChatDb.sessions.put(session);
				if (mapped.length > 0) await globalChatDb.messages.bulkPut(mapped);
			} catch (error) {
				console.warn(
					`FlowPilot history migration skipped conversation ${conv.id}:`,
					error,
				);
			}
		}

		localStorage.setItem(MIGRATION_FLAG, "1");
	} catch (error) {
		// Best-effort: leave the flag unset so a transient failure retries next launch.
		console.warn("FlowPilot history migration failed:", error);
	}
}
