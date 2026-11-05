import type { BotEmotionMessage } from "../../../lib/bot-emotion/protocol";
import type { IMessage } from "./chat-db";
import { isVisibleChatMessage } from "./chat-message-interaction";
import { extractThinkBlocks } from "./reasoning-format";

/** Only visible conversation text supplies context for the local facial reaction. */
export function botReactionMessages(
	messages: readonly IMessage[],
	sessionId?: string,
): BotEmotionMessage[] {
	const recent: BotEmotionMessage[] = [];
	const ordered = [...messages].sort((a, b) => b.timestamp - a.timestamp);
	for (const message of ordered) {
		const role = message.inner.role;
		if (
			(sessionId !== undefined && message.sessionId !== sessionId) ||
			(role !== "user" && role !== "assistant") ||
			!isVisibleChatMessage(message)
		)
			continue;
		const value = message.inner.content;
		const text =
			typeof value === "string"
				? value
				: value
						.filter((part) => part.type === "text")
						.map((part) => part.text ?? "")
						.join("");
		const content = (
			role === "assistant" ? extractThinkBlocks(text).text : text
		).trim();
		if (!content) continue;
		recent.push({ role, content: content.slice(-1200) });
		if (recent.length === 4) break;
	}
	return recent.reverse();
}
