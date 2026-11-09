import type { IChatMessageError } from "../../lib/flowpilot/chat-error";
import type { ChatErrorKind } from "../../lib/flowpilot/chat-error";
import { IRole } from "../../lib/schema/llm/history";
import { GLOBAL_CHAT_APP_ID } from "../../state/global-chat/global-chat-db";
import type { IMessage } from "../interfaces/chat-default/chat-db";
import type { ConversationMessage } from "./fold-entries";
import { type MentionTarget, inlineMentions } from "./mentions";

const ERROR_KINDS: Record<string, ChatErrorKind> = {
	BOT_TURN_MODEL_ERROR: "server",
	BOT_TURN_TIMEOUT: "network",
	BOT_TURN_QUOTA: "billing",
	BOT_TURN_SETUP: "config",
	BOT_TURN_REPLY_REFUSED: "permission",
};

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
		? {
				kind: ERROR_KINDS[message.errorCode] ?? "generic",
				title: options.errorTitle ?? "Couldn't answer",
				message: text,
				code: message.errorCode,
				retryable: false,
			}
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
