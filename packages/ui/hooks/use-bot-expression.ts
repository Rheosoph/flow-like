"use client";

import { useEffect, useRef, useState } from "react";
import { getBotEmotionClient } from "../lib/bot-emotion/client";
import {
	BotExpressionController,
	type BotExpressionState,
} from "../lib/bot-emotion/controller";
import {
	type EmotionMessage,
	boundEmotionInput,
} from "../lib/bot-emotion/protocol";

export interface UseBotExpressionOptions {
	draft: string;
	messages: readonly EmotionMessage[];
	conversationId: string;
	enabled?: boolean;
}

const REST: BotExpressionState = { expression: "neutral", status: "idle" };

export function useBotExpression({
	draft,
	messages,
	conversationId,
	enabled = true,
}: UseBotExpressionOptions): BotExpressionState {
	const [result, setResult] = useState({ conversationId, state: REST });
	const controller = useRef<
		{ conversationId: string; instance: BotExpressionController } | undefined
	>(undefined);
	const input = boundEmotionInput({ draft, messages });
	const inputKey = JSON.stringify(input);

	useEffect(() => {
		if (!enabled) return;
		const instance = new BotExpressionController(
			getBotEmotionClient(),
			(state) => setResult({ conversationId, state }),
		);
		controller.current = { conversationId, instance };
		return () => {
			instance.dispose();
			if (controller.current?.instance === instance)
				controller.current = undefined;
		};
	}, [conversationId, enabled]);

	useEffect(() => {
		if (enabled && controller.current?.conversationId === conversationId)
			controller.current.instance.update(JSON.parse(inputKey));
	}, [inputKey, conversationId, enabled]);

	if (!enabled || result.conversationId !== conversationId) return REST;
	return input.draft || result.state.lingering
		? result.state
		: { ...result.state, expression: "neutral" };
}
