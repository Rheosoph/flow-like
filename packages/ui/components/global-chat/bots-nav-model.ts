import type { IConversation } from "../../state/backend-state/inbox-state";
import {
	conversationBotId,
	currentConversationId,
} from "../../state/global-chat/global-chat-bots";
import {
	selectToolPromptConversationId,
	type useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import type { BotAgentState } from "../bots/types";
import { type BotChatActivity, botMessageActivity } from "./bots-nav-history";

type ChatState = ReturnType<typeof useGlobalChatStore.getState>;

export function latestBotActivity(
	...activities: (BotChatActivity | null | undefined)[]
): BotChatActivity | null {
	return activities.reduce<BotChatActivity | null>(
		(latest, activity) =>
			activity && (!latest || activity.at > latest.at) ? activity : latest,
		null,
	);
}

/** Pins, read receipts and room edits do not change which bot last exchanged a message. */
export function latestInboxBotActivity(
	conversations: readonly IConversation[],
): BotChatActivity | null {
	let latest: BotChatActivity | null = null;
	for (const conversation of conversations) {
		const entry = conversation.last_entry;
		if (!entry || entry.kind !== "message") continue;
		const bots = conversation.members.filter((member) => member.kind === "bot");
		const sender = bots.find((bot) => bot.principal_id === entry.sender_id);
		// A room's human message has no single bot recipient. Direct chats do.
		const bot =
			sender ??
			(conversation.kind === "direct" && bots.length === 1
				? bots[0]
				: undefined);
		const at = Date.parse(entry.created_at);
		if (!bot || !Number.isFinite(at) || at <= 0) continue;
		latest = latestBotActivity(latest, { botId: bot.principal_id, at });
	}
	return latest;
}

/** Includes background turns, without treating merely opening a chat as activity. */
export function latestLiveBotActivity(
	state: Pick<ChatState, "messages" | "runs">,
): BotChatActivity | null {
	let latest: BotChatActivity | null = null;
	for (const message of state.messages) {
		latest = latestBotActivity(latest, botMessageActivity(message));
	}
	for (const run of Object.values(state.runs)) {
		latest = latestBotActivity(
			latest,
			{
				botId: conversationBotId(currentConversationId(run.conversationId)),
				at: run.startedAt,
			},
			run.message && botMessageActivity(run.message),
		);
	}
	return latest;
}

export function navBotAgentState(
	state: Pick<ChatState, "runs" | "toolPrompt" | "activeConversationId">,
	botId: string | null,
): BotAgentState {
	const belongsToBot = (id: string) =>
		conversationBotId(currentConversationId(id)) === botId;
	if (
		state.toolPrompt &&
		belongsToBot(
			selectToolPromptConversationId(state) ?? state.activeConversationId,
		)
	)
		return "needs-you";
	return Object.values(state.runs).some((run) =>
		belongsToBot(run.conversationId),
	)
		? "working"
		: "idle";
}
