"use client";

import { useId } from "react";
import { useBotExpression } from "../../../hooks/use-bot-expression";
import type { BotEmotionMessage } from "../../../lib/bot-emotion/protocol";
import { BotAvatar } from "../../bots/bot-avatar";
import type { BotAgentState } from "../../bots/types";

export interface ChatBotReactionContext {
	conversationId: string;
	messages: readonly BotEmotionMessage[];
}

const NO_MESSAGES: readonly BotEmotionMessage[] = [];

/** A visual reaction stays local to the composer and never changes the submitted message. */
export function ChatComposerBot({
	draft,
	context,
	composing = false,
	agentState = "idle",
}: {
	draft: string;
	context?: ChatBotReactionContext;
	composing?: boolean;
	agentState?: BotAgentState;
}) {
	const instanceId = useId();
	const { expression } = useBotExpression({
		draft,
		messages: context?.messages ?? NO_MESSAGES,
		conversationId: context?.conversationId ?? instanceId,
		enabled: !composing,
	});

	return (
		<div className="pointer-events-none shrink-0 pl-3 pt-1" data-fl-chat-bot>
			<BotAvatar
				size={36}
				pose="idle"
				expression={expression}
				agentState={agentState}
				decorative
			/>
		</div>
	);
}
