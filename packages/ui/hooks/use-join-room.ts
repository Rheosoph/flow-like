"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import { useCallback } from "react";
import { toast } from "sonner";
import { attemptRoomJoin, roomJoinFailureKind } from "../lib/join-invite";
import { useBackend } from "../state/backend-state";
import { inboxKeys } from "./use-inbox";

/**
 * Joins a room from an invite link token, with retry, and toasts the outcome.
 * Resolves to the room's conversation id, or null when the join failed.
 */
export function useJoinRoomLink() {
	const { t } = useTranslation("common");
	const state = useBackend().inboxState;
	const queryClient = useQueryClient();
	return useCallback(
		async (
			token: string,
			onAttempt?: (attempt: number) => void,
		): Promise<string | null> => {
			if (!state) {
				toast.error(
					t(
						"joinRoom.signInRequired",
						"Sign in with a Flow-Like account to join this room.",
					),
				);
				return null;
			}
			const { result, joined } = await attemptRoomJoin(
				() => state.joinConversationLink(token),
				onAttempt,
			);
			if (result.ok && joined) {
				void queryClient.invalidateQueries({
					queryKey: inboxKeys.conversations(),
				});
				if (joined.joined)
					toast.success(t("joinRoom.joined", "You joined the room."));
				else toast.info(t("joinRoom.already", "You are already in this room."));
				return joined.conversation.id;
			}
			console.error("Failed to join room:", result.kind);
			switch (roomJoinFailureKind(result)) {
				case "used-up":
					toast.error(
						t("joinRoom.usedUp", "This room link has reached its usage limit."),
					);
					break;
				case "full":
					toast.error(
						t(
							"joinRoom.full",
							"This room is full, so you can't join it right now.",
						),
					);
					break;
				case "invalid":
					toast.error(
						t(
							"joinRoom.invalid",
							"This room link is invalid, expired, or was revoked.",
						),
					);
					break;
				default:
					toast.error(
						t(
							"joinRoom.retry",
							"Joining did not complete. Check your connection and open the room link again.",
						),
					);
			}
			return null;
		},
		[state, queryClient, t],
	);
}
