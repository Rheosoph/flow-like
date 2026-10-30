import type { IIntercomEvent } from "../../../lib/schema/events/intercom-event";
import { isRuntimeNamespace } from "../../../lib/service-runtime/session-scope";
import { recordChatActivity } from "../../../state/chat-activity-store";
import {
	type IMessage,
	chatDb,
	finalizeChatMessages,
	putChatDrafts,
	putChatMessage,
} from "./chat-db";
import type { IChatTurn } from "./chat-turn";
import { joinContentText } from "./inline-segments";

type AppChatTarget = { appId: string; eventId: string; sessionId: string };

/** Save the answer before making it available from the app-wide conversation inbox. */
export async function completeAppChatTurn(
	messages: readonly IMessage[],
	target: AppChatTarget,
	options?: { streaming?: boolean },
): Promise<void> {
	await chatDb.transaction("rw", chatDb.messages, chatDb.drafts, async () => {
		const existing = await chatDb.messages.bulkGet(
			messages.map((message) => message.id),
		);
		for (const [index, message] of messages.entries()) {
			// Snapshot and live-turn objects advance independently. Keep each saved revision
			// ahead of the last row so the transcript cannot reuse an earlier message body.
			message.rev = Math.max(message.rev ?? 0, existing[index]?.rev ?? 0);
		}
		if (options?.streaming) {
			for (const message of messages) await putChatMessage(message, { streaming: true });
		} else {
			await finalizeChatMessages(messages);
		}
	});
	if (
		isRuntimeNamespace(target.appId) ||
		messages.some((message) => message.error?.kind === "cancelled")
	)
		return;
	const session = await chatDb.sessions.get(target.sessionId);
	// A removed conversation has no destination in history, even if its run finishes later.
	if (!session || session.appId !== target.appId) return;
	for (let index = messages.length - 1; index >= 0; index--) {
		const message = messages[index];
		if (
			message.inner.role !== "assistant" ||
			message.error?.kind === "cancelled"
		)
			continue;
		const preview =
			joinContentText(message.inner.content).trim() ||
			message.error?.message ||
			(message.files.length > 0 ? "Shared an attachment" : "") ||
			(message.widgets?.length || message.main_widget?.widget
				? "Shared a widget"
				: "");
		if (!preview) continue;
		recordChatActivity({
			target: { kind: "app", ...target },
			messageId: message.id,
			title: session.summarization || message.explicit_name || "Chat",
			preview,
		});
		break;
	}
}

/** Complete response frames can arrive while a bot keeps running. Token chunks stay in its draft. */
export function createAppChatActivityWriter(
	turn: IChatTurn,
	target: AppChatTarget,
	getEvents: () => IIntercomEvent[],
) {
	let writes = Promise.resolve();
	return {
		settled: () => writes,
		onEvents(events: IIntercomEvent[]) {
			const pushed = events.findLast(
				(item) =>
					(item.event_type === "chat_stream" ||
						item.event_type === "chat_out") &&
					item.payload?.response,
			);
			if (!pushed) return;
			// Subscribers fold synchronously after onLiveEvents. The queued write catches up
			// only when they detached and finishes before the run's final save.
			writes = writes
				.then(async () => {
					turn.catchUp(getEvents());
					const snapshot = structuredClone(turn.responseMessages);
					const message = pushed.payload.message_id
						? snapshot.find(
								(candidate) =>
									candidate.stream_message_id === pushed.payload.message_id,
							)
						: snapshot.at(-1);
					if (!message) return;
					// Reopening before the next periodic checkpoint must retain the same root id.
					await putChatDrafts(snapshot);
					await completeAppChatTurn([message], target, {
						streaming: true,
					});
				})
				.catch((error) => {
					console.warn("[Chat] Could not save the pushed reply:", error);
				});
		},
	};
}
