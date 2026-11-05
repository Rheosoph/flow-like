"use client";

import { useTranslation } from "@flow-like/locales";
import { BanIcon } from "lucide-react";
import { useState } from "react";
import {
	useBlockPrincipal,
	useInboxBlocks,
	useUnblockPrincipal,
} from "../../hooks/use-inbox-blocks";
import { apiErrorMessage } from "../../lib/api-error";
import type {
	IBlockedPrincipal,
	IConversation,
} from "../../state/backend-state/inbox-state";
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
import { isSelfPrincipal } from "./crew-model";
import type { CrewDirectory } from "./use-crew-directory";

export interface BlockTarget {
	id: string;
	name: string;
	kind: "user" | "bot";
	ownerId?: string;
}

export function conversationBlockTarget(
	conversation: IConversation,
	directory: CrewDirectory,
	me?: string | null,
): BlockTarget | undefined {
	if (!me || conversation.local || conversation.kind !== "direct")
		return undefined;
	const member = [
		...conversation.members,
		...(conversation.pending ?? []),
	].find((member) => !isSelfPrincipal(member.principal_id, me));
	if (!member) return undefined;
	const person = directory(member.principal_id);
	if (
		member.kind === "bot" &&
		(!person.ownerId || isSelfPrincipal(person.ownerId, me))
	)
		return undefined;
	return {
		id: member.principal_id,
		name: person.name,
		kind: member.kind,
		ownerId: person.ownerId,
	};
}

export function principalBlock(
	blocks: readonly IBlockedPrincipal[] | undefined,
	principal: BlockTarget | undefined,
) {
	if (!principal) return undefined;
	return (
		blocks?.find((block) => block.principal_id === principal.id) ??
		(principal.kind === "bot"
			? blocks?.find((block) => block.principal_id === principal.ownerId)
			: undefined)
	);
}

export function PrincipalBlockDialog({
	principal,
	blocked = false,
	onOpenChange,
}: Readonly<{
	principal: BlockTarget;
	blocked?: boolean;
	onOpenChange: (open: boolean) => void;
}>) {
	const { t } = useTranslation("bots");
	const block = useBlockPrincipal();
	const unblock = useUnblockPrincipal();
	const [error, setError] = useState<string | null>(null);
	const [unblocking] = useState(blocked);
	const mutation = unblocking ? unblock : block;
	const submit = async () => {
		setError(null);
		try {
			await mutation.mutateAsync(principal.id);
			onOpenChange(false);
		} catch (cause) {
			setError(
				apiErrorMessage(
					cause,
					t("crew.blocks.failed", "Couldn't update the block. Try again."),
				),
			);
		}
	};
	return (
		<AlertDialog
			open
			onOpenChange={(open) => {
				if (!mutation.isPending) onOpenChange(open);
			}}
		>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>
						{unblocking
							? t("crew.blocks.unblockTitle", "Unblock {{name}}?", {
									name: principal.name,
								})
							: t("crew.blocks.blockTitle", "Block {{name}}?", {
									name: principal.name,
								})}
					</AlertDialogTitle>
					<AlertDialogDescription>
						{unblocking
							? t(
									"crew.blocks.unblockDescription",
									"This block will be removed. Other blocks will still apply.",
								)
							: principal.kind === "user"
								? t(
										"crew.blocks.userDescription",
										"Direct messages and invitations between you and this person, including your bots and theirs, will be blocked. You will still see each other in shared rooms. You can leave those rooms separately.",
									)
								: t(
										"crew.blocks.botDescription",
										"Direct messages and invitations between you, your bots and this bot will be blocked. The bot will remain in shared rooms. You can leave those rooms separately.",
									)}
					</AlertDialogDescription>
				</AlertDialogHeader>
				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
				<AlertDialogFooter>
					<AlertDialogCancel disabled={mutation.isPending}>
						{t("crew.cancel", "Cancel")}
					</AlertDialogCancel>
					<Button
						type="button"
						variant={unblocking ? "default" : "destructive"}
						disabled={mutation.isPending}
						onClick={() => void submit()}
					>
						{unblocking
							? t("crew.blocks.unblock", "Unblock")
							: t("crew.blocks.block", "Block")}
					</Button>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}

export function PrincipalBlockButton({
	principal,
}: Readonly<{ principal: BlockTarget }>) {
	const { t } = useTranslation("bots");
	const blocks = useInboxBlocks();
	const existing = principalBlock(blocks.data, principal);
	const inherited = existing && existing.principal_id !== principal.id;
	const [open, setOpen] = useState(false);
	return (
		<>
			<Button
				type="button"
				variant="ghost"
				size="sm"
				disabled={!blocks.data || Boolean(inherited)}
				onClick={() => setOpen(true)}
				aria-label={
					inherited
						? t("crew.blocks.ownerBlocked", "Owner blocked")
						: existing
							? t("crew.blocks.unblockName", "Unblock {{name}}", {
									name: principal.name,
								})
							: t("crew.blocks.blockName", "Block {{name}}", {
									name: principal.name,
								})
				}
			>
				<BanIcon aria-hidden />
				{inherited
					? t("crew.blocks.ownerBlocked", "Owner blocked")
					: existing
						? t("crew.blocks.unblock", "Unblock")
						: t("crew.blocks.block", "Block")}
			</Button>
			{open && (
				<PrincipalBlockDialog
					principal={principal}
					blocked={Boolean(existing)}
					onOpenChange={setOpen}
				/>
			)}
		</>
	);
}
