"use client";

import { useChat as useBaseChat } from "@ai-sdk/react";
import { AIChatPlugin, type AIChatPluginConfig } from "@platejs/ai/react";
import { useEditorRef } from "platejs/react";
import * as React from "react";

import { useBackend } from "../../state/backend-state";
import { BackendEditorChatTransport } from "./ai-transport";
import { AIUsageAppContext } from "./ai-usage-context";

export type EditorChat = AIChatPluginConfig["options"]["chat"];
export type EditorChatMessage = EditorChat["messages"][number];

/** Plate's copilot still checks `chat.isLoading`, which AI SDK 5 removed, to stay quiet while the menu streams. */
type PublishedEditorChat = EditorChat & { isLoading: boolean };

/**
 * The editor's AI chat, published to `AIChatPlugin`'s `chat` option for Plate's submit,
 * streaming and menu hooks. Each editor owns its own conversation.
 */
export const useChat = () => {
	const editor = useEditorRef();
	const { aiState } = useBackend();
	const appId = React.useContext(AIUsageAppContext);
	const transport = React.useMemo(
		() => new BackendEditorChatTransport<EditorChatMessage>({ aiState, appId }),
		[aiState, appId],
	);
	const chat = useBaseChat<EditorChatMessage>({ id: "editor", transport });

	// biome-ignore lint/correctness/useExhaustiveDependencies: `chat` is a new object every render and the option feeds back into this component, so publish only when its state changes.
	React.useEffect(() => {
		const published: PublishedEditorChat = {
			...chat,
			isLoading: chat.status === "submitted" || chat.status === "streaming",
		};
		editor.setOption(AIChatPlugin, "chat", published);
	}, [editor, chat.status, chat.messages, chat.error]);

	return chat;
};
