"use client";

import { useCallback } from "react";
import { IRole, Response, useBackend } from "../../index";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import {
	makeGlobalChatMessage,
	persistGlobalChatMessage,
} from "../../state/global-chat/global-chat-stream";
import type { IAttachment } from "../interfaces/chat-default/chat-db";
import type { RunWidgetAction } from "../interfaces/chat-default/chat-widget-execution";
import { processChatEvents } from "../interfaces/chat-default/event-processor";

/**
 * Widget-action runner for widgets embedded in global-chat messages. The
 * clicked widget carries the pushing run's `origin`, so ActionHandler hands
 * this the ORIGINAL use-case app/board — the action executes there as a plain
 * board run starting at the bound node. The run's a2ui events (forwarded via
 * `onA2UIEvents`) update the widget in place; any chat content the triggered
 * workflow pushes is appended to the global conversation as a new assistant
 * message once the run completes; interactions it raises render inline.
 */
export function useGlobalChatRunWidgetAction(): RunWidgetAction {
	const backend = useBackend();

	return useCallback<RunWidgetAction>(
		async (appId, boardId, runPayload, onA2UIEvents) => {
			const responseMessage = makeGlobalChatMessage(
				IRole.Assistant,
				"",
				useGlobalChatStore.getState().activeConversationId,
			);
			let intermediateResponse = Response.default();
			const attachments = new Map<string, IAttachment>();

			const result = await backend.boardState.executeBoard(
				appId,
				boardId,
				runPayload,
				false,
				undefined,
				(events) => {
					onA2UIEvents?.(events);

					const processed = processChatEvents(events, {
						intermediateResponse,
						responseMessage,
						attachments,
						tmpLocalState: null,
						tmpGlobalState: null,
						done: false,
						appId,
						eventId: "",
						sessionId: responseMessage.sessionId,
					});
					intermediateResponse = processed.intermediateResponse;

					if (processed.interactions?.length) {
						useGlobalChatStore
							.getState()
							.addInteractions(processed.interactions);
					}
				},
			);

			// Widgets pushed by the action run stay bound to the board that ran it,
			// so their own actions keep routing to the original use-case board.
			if (responseMessage.widgets?.length) {
				responseMessage.widgets = responseMessage.widgets.map((widget) => ({
					...widget,
					origin: { appId, boardId },
				}));
			}

			const textContent =
				typeof responseMessage.inner.content === "string"
					? responseMessage.inner.content.trim()
					: (responseMessage.inner.content?.length ?? 0);
			const hasContent = Boolean(
				textContent ||
					responseMessage.files?.length ||
					responseMessage.widgets?.length ||
					responseMessage.plan_steps?.length,
			);

			// Only surface a new assistant message when the action produced chat
			// content; a pure in-place widget update leaves no residue.
			if (hasContent) {
				useGlobalChatStore.getState().appendMessage(responseMessage);
				void persistGlobalChatMessage(responseMessage);
			}

			return result;
		},
		[backend.boardState],
	);
}
