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
import { Input } from "../../../components/ui/input";
import { useInvalidateInvoke } from "../../../hooks";
import { updateNodeCommand } from "../../../lib";
import type { INode } from "../../../lib/schema/flow/node";
import { useBackend } from "../../../state/backend-state";
import { useUndoRedo } from "../flow-history";

export function FlowNodeRenameMenu({
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
	const [friendlyName, setFriendlyName] = useState(node.friendly_name);

	async function saveComment() {
		const command = updateNodeCommand({
			node: { ...node, friendly_name: friendlyName },
		});

		const result = await backend.boardState.executeCommand(
			appId,
			boardId,
			command,
		);
		await pushCommand(result);
		onOpenChange(false);
		setFriendlyName("");
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
					<DialogTitle>{t('renameNode', 'Rename Node')}</DialogTitle>
				</DialogHeader>
				<DialogDescription>
					<Input
						value={friendlyName}
						onChange={(e) => {
							setFriendlyName(e.target.value);
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
						{t('cancel', 'Cancel')}
					</Button>
					<Button onClick={async () => await saveComment()}>Save</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
