"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback } from "react";
import { toast } from "sonner";
import { useBackend } from "../../state/backend-state";
import { getFrontendStateStore } from "../a2ui/frontend-state";
import type { RunWidgetAction } from "../interfaces/chat-default/chat-widget-execution";

/**
 * Widget actions in a room run the widget's origin board as the person who clicks, with their own
 * permissions. The run's events update the clicked widget in place; what the run pushes to chat is
 * not posted to the room.
 */
export function useRoomRunWidgetAction(): RunWidgetAction {
	const backend = useBackend();
	const { t } = useTranslation("bots");

	return useCallback<RunWidgetAction>(
		async (appId, boardId, runPayload, onA2UIEvents) => {
			try {
				return await backend.boardState.executeBoard(
					appId,
					boardId,
					runPayload,
					false,
					undefined,
					(events) => {
						if (onA2UIEvents) {
							onA2UIEvents(events);
							return;
						}
						for (const item of events) {
							if (item.event_type === "a2ui")
								getFrontendStateStore(appId).handleMessage(item.payload);
						}
					},
				);
			} catch (error) {
				console.warn(
					`[rooms] Running the widget action on board ${boardId} of app ${appId} failed`,
					error,
				);
				toast.error(
					t(
						"crew.messages.widgetActionFailed",
						"The app couldn't run this action for you.",
					),
				);
				return undefined;
			}
		},
		[backend.boardState, t],
	);
}
