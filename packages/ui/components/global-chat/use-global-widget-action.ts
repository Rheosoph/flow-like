"use client";

import { useCallback } from "react";
import { IRole, useBackend } from "../../index";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import {
	makeGlobalChatMessage,
	persistGlobalChatMessage,
} from "../../state/global-chat/global-chat-stream";
import { getFrontendStateStore } from "../a2ui/frontend-state";
import { createChatTurn } from "../interfaces/chat-default/chat-turn";
import type { RunWidgetAction } from "../interfaces/chat-default/chat-widget-execution";
import {
	refreshMainChatWidget,
	resolveMainChatWidget,
} from "../interfaces/chat-default/main-widget-state";

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
			let responseMessage = makeGlobalChatMessage(
				IRole.Assistant,
				"",
				useGlobalChatStore.getState().activeConversationId,
			);
			const currentMainWidget = () => {
				const state = useGlobalChatStore.getState();
				return resolveMainChatWidget(
					[
						...state.messages,
						...Object.values(state.runs).flatMap((run) =>
							run.message ? [run.message] : [],
						),
					],
					responseMessage.sessionId,
				);
			};
			responseMessage.main_widget = currentMainWidget();
			const initialMainWidget = responseMessage.main_widget;
			let publishedMainWidget = initialMainWidget;
			const turn = createChatTurn({
				responseMessage,
				tmpLocalState: null,
				tmpGlobalState: null,
				appId: responseMessage.appId,
				eventId: "",
				sessionId: responseMessage.sessionId,
			});

			const result = await backend.boardState.executeBoard(
				appId,
				boardId,
				runPayload,
				false,
				undefined,
				(events) => {
					if (!onA2UIEvents) {
						for (const item of events) {
							if (item.event_type === "a2ui") {
								getFrontendStateStore(appId).handleMessage(item.payload);
							}
						}
					}
					onA2UIEvents?.(events);

					refreshMainChatWidget(turn.responseMessage, currentMainWidget());
					const processed = turn.fold(events);
					responseMessage = processed.responseMessage;
					if (responseMessage.main_widget !== publishedMainWidget) {
						const mainWidget = responseMessage.main_widget;
						if (mainWidget?.widget && !mainWidget.widget.origin) {
							responseMessage.main_widget = {
								...mainWidget,
								widget: { ...mainWidget.widget, origin: { appId, boardId } },
							};
						}
						publishedMainWidget = responseMessage.main_widget;
						useGlobalChatStore.getState().commitMessage({ ...responseMessage });
					}

					if (processed.interactions?.length) {
						useGlobalChatStore
							.getState()
							.addInteractions(processed.interactions);
					}
				},
			);

			// Widgets pushed by the action run stay bound to the board that ran it,
			// so their own actions keep routing to the original use-case board.
			for (const message of turn.responseMessages) {
				if (message.main_widget?.widget && !message.main_widget.widget.origin) {
					message.main_widget = {
						...message.main_widget,
						widget: {
							...message.main_widget.widget,
							origin: { appId, boardId },
						},
					};
				}
				if (message.widgets?.length) {
					message.widgets = message.widgets.map((widget) => ({
						...widget,
						origin: { appId, boardId },
					}));
				}

				const textContent =
					typeof message.inner.content === "string"
						? message.inner.content.trim()
						: (message.inner.content?.length ?? 0);
				const hasContent = Boolean(
					textContent ||
						message.files?.length ||
						message.widgets?.length ||
						message.main_widget !== initialMainWidget ||
						message.plan_steps?.length,
				);

				// Save pane changes even when the action did not produce a chat bubble.
				if (hasContent) {
					useGlobalChatStore.getState().commitMessage(message);
					await persistGlobalChatMessage(message);
				}
			}

			return result;
		},
		[backend.boardState],
	);
}
