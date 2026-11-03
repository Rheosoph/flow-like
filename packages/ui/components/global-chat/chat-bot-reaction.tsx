"use client";

import {
	type ReactNode,
	createContext,
	useContext,
	useEffect,
	useRef,
	useState,
} from "react";
import { useStore } from "zustand";
import { createStore } from "zustand/vanilla";
import type { BotAgentState, BotExpression } from "../bots/types";

interface BotReaction {
	expression: BotExpression;
	agentState: BotAgentState;
	expressionOverridesState: boolean;
}

const REST: BotReaction = {
	expression: "neutral",
	agentState: "idle",
	expressionOverridesState: false,
};

function createReactionStore() {
	return createStore<{
		current: {
			owner: object;
			botId: string | null;
			reaction: BotReaction;
		} | null;
	}>()(() => ({ current: null }));
}

type ReactionStore = ReturnType<typeof createReactionStore>;
const ReactionContext = createContext<ReactionStore | null>(null);
const EMPTY_STORE = createReactionStore();

/** Shares the header's computed reaction with the selected crew portrait, without retaining text. */
export function ChatBotReactionProvider({ children }: { children: ReactNode }) {
	const [store] = useState(createReactionStore);
	return (
		<ReactionContext.Provider value={store}>
			{children}
		</ReactionContext.Provider>
	);
}

export function usePublishChatBotReaction(
	botId: string | null | undefined,
	expression: BotExpression,
	agentState: BotAgentState,
	expressionOverridesState: boolean,
) {
	const store = useContext(ReactionContext);
	const owner = useRef({}).current;
	useEffect(() => {
		if (!store || botId === undefined) return;
		return () => {
			if (store.getState().current?.owner === owner)
				store.setState({ current: null });
		};
	}, [store, owner, botId]);
	useEffect(() => {
		if (!store || botId === undefined) return;
		store.setState({
			current: {
				owner,
				botId,
				reaction: { expression, agentState, expressionOverridesState },
			},
		});
	}, [store, owner, botId, expression, agentState, expressionOverridesState]);
}

/** Only the selected bot reads the live reaction; every other portrait keeps its resting face. */
export function useChatBotReaction(
	botId: string | null | undefined,
): BotReaction {
	const store = useContext(ReactionContext) ?? EMPTY_STORE;
	return useStore(store, (state) =>
		botId !== undefined && state.current?.botId === botId
			? state.current.reaction
			: REST,
	);
}
