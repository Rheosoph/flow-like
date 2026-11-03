"use client";

import {
	useCallback,
	useEffect,
	useId,
	useMemo,
	useRef,
	useState,
} from "react";
import { useBotExpression } from "../../hooks/use-bot-expression";
import type { BotEmotionMessage } from "../../lib/bot-emotion/protocol";
import {
	selectActiveBotId,
	selectToolPromptHere,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import { BotAvatar } from "../bots/bot-avatar";
import type { BotAgentState, BotAppearance } from "../bots/types";
import { botReactionMessages } from "../interfaces/chat-default/bot-reaction-context";
import { usePublishChatBotReaction } from "./chat-bot-reaction";

export interface ChatBotReactionContext {
	conversationId: string;
	messages: readonly BotEmotionMessage[];
}

const NO_MESSAGES: readonly BotEmotionMessage[] = [];

/** The bot a chat talks to. Its face follows the draft and the latest messages, read on this device. */
export function ChatBotAvatar({
	draft,
	context,
	agentState = "idle",
	appearance,
	size,
	className,
	botId,
}: Readonly<{
	draft: string;
	context?: ChatBotReactionContext;
	agentState?: BotAgentState;
	appearance?: Partial<BotAppearance>;
	size: number;
	className?: string;
	botId?: string | null;
}>) {
	const instanceId = useId();
	const { expression, lingering } = useBotExpression({
		draft,
		messages: context?.messages ?? NO_MESSAGES,
		conversationId: context?.conversationId ?? instanceId,
	});
	const expressionOverridesState =
		Boolean(draft.trim()) ||
		Boolean(lingering && (agentState === "idle" || agentState === "working"));
	usePublishChatBotReaction(
		botId,
		expression,
		agentState,
		expressionOverridesState,
	);
	return (
		<BotAvatar
			appearance={appearance}
			size={size}
			pose="idle"
			expression={expression}
			expressionOverridesState={expressionOverridesState}
			agentState={agentState}
			decorative
			className={className}
		/>
	);
}

type DraftListener = (draft: string) => void;
export interface DraftRelay {
	current: DraftListener | null;
	draft: string;
}

/** Hands the composer's draft to the pill's avatar without re-rendering the chat on every keystroke. */
export function useDraftRelay() {
	const listener = useRef<DraftRelay>({ current: null, draft: "" }).current;
	const report = useCallback(
		(draft: string) => {
			listener.draft = draft;
			listener.current?.(draft);
		},
		[listener],
	);
	return { listener, report };
}

/** The avatar in a bot chat's pill: reacts to the draft and shows when the bot works or waits for you. */
export function GlobalChatBotAvatar({
	relay,
	appearance,
	size,
	className,
}: Readonly<{
	relay: DraftRelay;
	appearance?: Partial<BotAppearance>;
	size: number;
	className?: string;
}>) {
	const [draft, setDraft] = useState(relay.draft);
	useEffect(() => {
		relay.current = setDraft;
		setDraft(relay.draft);
		return () => {
			if (relay.current === setDraft) relay.current = null;
		};
	}, [relay]);
	const agentState = useGlobalChatStore((s) =>
		selectToolPromptHere(s) ? "needs-you" : s.isStreaming ? "working" : "idle",
	);
	const conversationId = useGlobalChatStore((s) => s.activeConversationId);
	const botId = useGlobalChatStore(selectActiveBotId);
	const messages = useGlobalChatStore((s) => s.messages);
	const context = useMemo(
		() => ({
			conversationId,
			messages: botReactionMessages(messages, conversationId),
		}),
		[conversationId, messages],
	);
	return (
		<ChatBotAvatar
			botId={botId}
			draft={draft}
			context={context}
			agentState={agentState}
			appearance={appearance}
			size={size}
			className={className}
		/>
	);
}
