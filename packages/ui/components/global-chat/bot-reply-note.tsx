"use client";

import { IRole } from "../../lib/schema/llm/history";
import { useBotHistoryStore } from "../../state/global-chat/bot-history-store";
import { activeBotTarget } from "../../state/global-chat/bot-inbox";
import { BotNoteChip } from "../bots-tab/bot-note-chip";
import type { IMessage } from "../interfaces/chat-default/chat-db";

/**
 * The note chip above a bot reply in the dock: the note an inbox reply carries, or for a reply
 * that streamed here the note its stored copy carried when it arrived.
 */
export function BotReplyNote({
	message,
	botId,
	sessionId,
}: Readonly<{ message: IMessage; botId: string; sessionId: string }>) {
	const answered = message.run_context?.user_message_id;
	const pushed = useBotHistoryStore((state) =>
		answered ? state.replyNotes[answered] : undefined,
	);
	const note = message.note ?? pushed;
	const conversationId = activeBotTarget(sessionId)?.conversationId;
	if (!note || !conversationId || message.inner.role !== IRole.Assistant) {
		return null;
	}
	return (
		<div className="mb-1.5">
			<BotNoteChip note={note} botId={botId} conversationId={conversationId} />
		</div>
	);
}
