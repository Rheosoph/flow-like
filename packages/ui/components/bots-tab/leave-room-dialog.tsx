"use client";

import { useTranslation } from "@flow-like/locales";
import { useState } from "react";
import { useRemoveConversationMember } from "../../hooks/use-inbox";
import { apiErrorMessage } from "../../lib/api-error";
import {
	AlertDialog,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../ui/alert-dialog";
import { Button } from "../ui/button";

export function LeaveRoomDialog({
	open,
	onOpenChange,
	conversationId,
	principalId,
	onLeave,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	conversationId: string;
	principalId: string;
	onLeave?: () => void;
}>) {
	const { t } = useTranslation("bots");
	const remove = useRemoveConversationMember();
	const [error, setError] = useState<string | null>(null);
	const leave = async () => {
		setError(null);
		try {
			await remove.mutateAsync({ conversationId, principalId });
			onOpenChange(false);
			onLeave?.();
		} catch (cause) {
			setError(
				apiErrorMessage(
					cause,
					t("crew.leaveRoom.failed", "Couldn't leave the room. Try again."),
				),
			);
		}
	};
	return (
		<AlertDialog
			open={open}
			onOpenChange={(next) => {
				if (remove.isPending) return;
				setError(null);
				onOpenChange(next);
			}}
		>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>
						{t("crew.leaveRoom.title", "Leave this room?")}
					</AlertDialogTitle>
					<AlertDialogDescription>
						{t(
							"crew.leaveRoom.description",
							"You and your bots in this room will leave and stop receiving its messages.",
						)}
					</AlertDialogDescription>
				</AlertDialogHeader>
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
				<AlertDialogFooter>
					<AlertDialogCancel disabled={remove.isPending}>
						{t("crew.cancel", "Cancel")}
					</AlertDialogCancel>
					<Button
						type="button"
						variant="destructive"
						disabled={remove.isPending}
						onClick={() => void leave()}
					>
						{t("crew.leaveRoom.action", "Leave room")}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
