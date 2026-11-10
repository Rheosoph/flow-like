import type { IChatMessageError } from "../../lib/flowpilot/chat-error";
import { IRole } from "../../lib/schema/llm/history";
import { GLOBAL_CHAT_APP_ID } from "../../state/global-chat/global-chat-db";
import type { IMessage } from "../interfaces/chat-default/chat-db";
import { botTurnError } from "./bot-turn-error";
import type { ConversationMessage } from "./fold-entries";
import { type MentionTarget, inlineMentions } from "./mentions";

/**
 * A room message as the chat engine reads it: yours is the ask, everyone else's an answer, so a
 * room renders text, reasoning, UI fences, steps, errors and approvals exactly like a bot's own
 * chat. Your reactions are `reactions`; `reactionSummary` carries counts and who reacted.
 */
export function roomChatMessage(
	message: ConversationMessage,
	conversationId: string,
	options: {
		mentions?: readonly MentionTarget[];
		mentionCandidates?: readonly MentionTarget[];
		errorTitle?: string;
		/** The sender is one of the reader's bots, so a failure they can fix offers the fix. */
		ownBot?: boolean;
		notice?: string;
		editedLabel?: string;
		reactorName?: (id: string) => string;
		replyTarget?: ConversationMessage;
		files?: IMessage["files"];
		storageBotId?: string;
	} = {},
): IMessage {
	const text =
		message.content.kind === "text"
			? message.content.text
			: (options.notice ?? "");
	const error: IChatMessageError | undefined = message.errorCode
		? botTurnError(message.errorCode, text, {
				title: options.errorTitle ?? "Couldn't answer",
				ownBot: options.ownBot ?? false,
			})
		: undefined;
	const mine = message.reactions.filter((r) => r.mine).map((r) => r.emoji);
	const reactionSummary = Object.fromEntries(
		message.reactions.map((r) => [
			r.emoji,
			{
				count: r.count,
				names: r.reactorIds.map((id) => options.reactorName?.(id) ?? id),
			},
		]),
	);
	const target = options.replyTarget;
	const interaction: IMessage["interaction"] =
		target?.content.kind === "text"
			? {
					type: "reply",
					target: {
						messageId: target.id,
						role: target.mine ? IRole.User : IRole.Assistant,
						text: target.content.text,
						...(target.userContextDerived ? { userContextDerived: true } : {}),
					},
				}
			: undefined;
	return {
		id: message.id,
		appId: GLOBAL_CHAT_APP_ID,
		sessionId: conversationId,
		...(options.storageBotId ? { storageBotId: options.storageBotId } : {}),
		inner: {
			role: message.mine ? IRole.User : IRole.Assistant,
			content: error
				? ""
				: inlineMentions(
						text,
						options.mentions ?? [],
						options.mentionCandidates,
					) +
					(message.edited && options.editedLabel
						? `\n\n*${options.editedLabel}*`
						: ""),
		},
		...(error ? { error } : {}),
		...(message.userContext ? { userContext: message.userContext } : {}),
		...(message.userContextDerived ? { userContextDerived: true } : {}),
		...(message.userContextDerivedFrom
			? { userContextDerivedFrom: message.userContextDerivedFrom }
			: {}),
		...(message.approval
			? {
					approval: {
						...message.approval,
						botId: message.senderId,
					},
				}
			: {}),
		reactions: mine,
		reactionSummary,
		...(message.content.kind === "text" ? { source_text: text } : {}),
		...(interaction ? { interaction } : {}),
		files: options.files ?? [],
		tools: [],
		actions: [],
		timestamp: message.time,
	};
}
