import { useTranslation } from "@flow-like/locales";
import { useState } from "react";
import { Button } from "../../../components/ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "../../../components/ui/dialog";
import { Textarea } from "../../../components/ui/textarea";
import { useInvalidateInvoke } from "../../../hooks";
import { updateNodeCommand } from "../../../lib";
import type { INode } from "../../../lib/schema/flow/node";
import { useBackend } from "../../../state/backend-state";
import { useUndoRedo } from "../flow-history";

export function FlowNodeCommentMenu({
	node,
	boardId,
	appId,
	open,
	onOpenChange,
}: Readonly<{
	node: INode;
	appId: string;
	boardId: string;
	open: boolean;
	onOpenChange: (open: boolean) => void;
}>) {
	const { t } = useTranslation("flow");
	const { pushCommand } = useUndoRedo(appId, boardId);
	const invalidate = useInvalidateInvoke();
	const backend = useBackend();
	const [comment, setComment] = useState("");

	async function saveComment() {
		const command = updateNodeCommand({
			node: { ...node, comment },
		});

		const result = await backend.boardState.executeCommand(
			appId,
			boardId,
			command,
		);
		await pushCommand(result);
		onOpenChange(false);
		setComment("");
		refetchBoard();
	}

	async function refetchBoard() {
		await invalidate(backend.boardState.getBoard, [appId, boardId]);
	}

	return (
		<Dialog
			open={open}
			onOpenChange={(open) => {
				onOpenChange(open);
			}}
		>
			<DialogContent>
				<DialogHeader>
					<DialogTitle>{t("comment", "Comment")}</DialogTitle>
				</DialogHeader>
				<DialogDescription>
					<Textarea
						rows={6}
						value={comment}
						onChange={(e) => {
							setComment(e.target.value);
						}}
					/>
				</DialogDescription>
				<DialogFooter>
					<Button
						onClick={() => {
							onOpenChange(false);
						}}
						variant={"secondary"}
					>
						{t("cancel", "Cancel")}
					</Button>
					<Button onClick={async () => await saveComment()}>Save</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
