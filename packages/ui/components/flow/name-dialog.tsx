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
	Input,
} from "../ui";

export function NameDialog({
	open,
	onOpenChange,
	name,
	onUpsert,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	name: string;
	onUpsert: (comment: string) => void;
}>) {
	const { t } = useTranslation("flow");
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent
				onClick={(e) => {
					e.stopPropagation();
					e.preventDefault();
				}}
			>
				<DialogHeader>
					<DialogTitle>Name</DialogTitle>
					<DialogDescription>
						{t("nameYourNode", "Name your node.")}
					</DialogDescription>
				</DialogHeader>
				<DialogDescription>
					<Input
						onClick={(e) => {
							e.stopPropagation();
							e.preventDefault();
						}}
						value={name}
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
