"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Button,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	Textarea,
} from "../ui";

export function CommentDialog({
	open,
	onOpenChange,
	comment,
	onUpsert,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	comment: string;
	onUpsert: (comment: string) => void;
}>) {
	const { t } = useTranslation("flow");
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent>
				<DialogHeader>
					<DialogTitle>{t("comment", "Comment")}</DialogTitle>
					<DialogDescription>
						{t("addACommentToTheNode", "Add a comment to the node.")}
					</DialogDescription>
				</DialogHeader>
				<DialogDescription>
					<Textarea
						value={comment}
						rows={6}
						onChange={(e) => onUpsert(e.target.value)}
					/>
				</DialogDescription>
				<DialogFooter>
					<Button
						className="w-full"
						onClick={() => {
							onOpenChange(false);
						}}
					>
						{t("save", "Save")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
