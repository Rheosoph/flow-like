"use client";

import { useTranslation } from "@flow-like/locales";
import { useState } from "react";
import { useDeleteConversation } from "../../hooks/use-inbox";
import { apiErrorMessage } from "../../lib/api-error";
import type { ConversationKind } from "../../state/backend-state/inbox-state";
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

export function DeleteConversationDialog({
	open,
	onOpenChange,
	conversationId,
	kind,
	onDelete,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	conversationId: string;
	kind: ConversationKind;
	onDelete?: () => void;
}>) {
	const { t } = useTranslation("bots");
	const remove = useDeleteConversation();
	const [error, setError] = useState<string | null>(null);
	const group = kind === "group";
	const confirm = async () => {
		setError(null);
		try {
			await remove.mutateAsync(conversationId);
			onOpenChange(false);
			onDelete?.();
		} catch (cause) {
			setError(
				apiErrorMessage(
					cause,
					t(
						"crew.deleteConversation.failed",
						"Couldn't delete the conversation. Try again.",
					),
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
						{group
							? t(
									"crew.deleteConversation.groupTitle",
									"Leave and delete this room?",
								)
							: t("crew.deleteConversation.title", "Delete this conversation?")}
					</AlertDialogTitle>
					<AlertDialogDescription>
						{group
							? t(
									"crew.deleteConversation.groupDescription",
									"You and your bots will leave this room and stop receiving its messages. The room and its history will be removed from your view. Other participants keep their copies. This cannot be undone.",
								)
							: t(
									"crew.deleteConversation.description",
									"This conversation and its history will be removed from your view. Other participants keep their copies. A new message or starting this chat again brings it back with your old history still cleared. This cannot be undone.",
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
						onClick={() => void confirm()}
					>
						{remove.isPending
							? t("crew.deleteConversation.pending", "Deleting…")
							: group
								? t("crew.deleteConversation.groupAction", "Leave and delete")
								: t("crew.deleteConversation.action", "Delete conversation")}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
