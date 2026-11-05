import { IRole } from "../../lib/schema/llm/history";
import {
	conversationBotId,
	currentConversationId,
} from "../../state/global-chat/global-chat-bots";
import {
	GLOBAL_CHAT_APP_ID,
	type IMessage,
	globalChatDb,
} from "../../state/global-chat/global-chat-db";

export interface BotChatActivity {
	botId: string | null;
	at: number;
}

function validTime(value: unknown): value is number {
	return typeof value === "number" && Number.isFinite(value) && value > 0;
}

/** A message counts when sent; a reply also counts when its run finishes. */
export function botMessageActivity(message: IMessage): BotChatActivity | null {
	const role = message.inner?.role;
	if (
		message.appId !== GLOBAL_CHAT_APP_ID ||
		message.notice ||
		(role !== IRole.User && role !== IRole.Assistant) ||
		typeof message.sessionId !== "string" ||
		!message.sessionId.trim() ||
		!validTime(message.timestamp)
	) {
		return null;
	}
	const endedAt = message.run_context?.ended_at_ms;
	return {
		botId: conversationBotId(currentConversationId(message.sessionId)),
		at:
			role === IRole.Assistant && validTime(endedAt)
				? Math.max(message.timestamp, endedAt)
				: message.timestamp,
	};
}

/** Indexed reads include older runs that finished after a more recently sent message. */
export async function readLatestBotChatActivity(): Promise<BotChatActivity | null> {
	const [sent, finished] = await Promise.all([
		globalChatDb.messages
			.orderBy("timestamp")
			.reverse()
			.filter((message) => botMessageActivity(message) !== null)
			.first(),
		globalChatDb.messages
			.orderBy("run_context.ended_at_ms")
			.reverse()
			.filter(
				(message) =>
					message.inner?.role === IRole.Assistant &&
					validTime(message.run_context?.ended_at_ms) &&
					botMessageActivity(message) !== null,
			)
			.first(),
	]);
	const sentActivity = sent ? botMessageActivity(sent) : null;
	const finishedActivity = finished ? botMessageActivity(finished) : null;
	if (!sentActivity) return finishedActivity;
	return finishedActivity && finishedActivity.at > sentActivity.at
		? finishedActivity
		: sentActivity;
}
