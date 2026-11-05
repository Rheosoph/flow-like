"use client";

import { useTranslation } from "@flow-like/locales";
import { BanIcon } from "lucide-react";
import { useMemo, useState } from "react";
import { useInboxBlocks } from "../../hooks/use-inbox-blocks";
import { IdentityAvatar } from "../identity/identity-avatar";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { PrincipalBlockButton } from "./principal-block-action";
import { useCrewDirectory } from "./use-crew-directory";

export function BlockedPrincipalsButton() {
	const { t } = useTranslation("bots");
	const [open, setOpen] = useState(false);
	return (
		<>
			<Button
				type="button"
				variant="outline"
				size="icon"
				className="size-10 shrink-0 rounded-[10px]"
				onClick={() => setOpen(true)}
				aria-label={t("crew.blocks.title", "Blocked users and bots")}
				title={t("crew.blocks.title", "Blocked users and bots")}
			>
				<BanIcon aria-hidden />
			</Button>
			{open && <BlockedPrincipalsDialog onOpenChange={setOpen} />}
		</>
	);
}

function BlockedPrincipalsDialog({
	onOpenChange,
}: Readonly<{ onOpenChange: (open: boolean) => void }>) {
	const { t } = useTranslation("bots");
	const blocks = useInboxBlocks();
	const ids = useMemo(
		() => blocks.data?.map((block) => block.principal_id) ?? [],
		[blocks.data],
	);
	const directory = useCrewDirectory(ids, {});
	return (
		<Dialog open onOpenChange={onOpenChange}>
			<DialogContent>
				<DialogHeader>
					<DialogTitle>
						{t("crew.blocks.title", "Blocked users and bots")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"crew.blocks.description",
							"Manage who can exchange direct messages and invitations with you and your bots. Shared rooms are unchanged.",
						)}
					</DialogDescription>
				</DialogHeader>
				{blocks.isPending && (
					<output className="text-sm text-muted-foreground">
						{t("crew.blocks.loading", "Loading blocked users and bots…")}
					</output>
				)}
				{blocks.isError && (
					<div
						role="alert"
						className="flex items-center justify-between gap-3 text-sm"
					>
						<span>
							{t("crew.blocks.loadFailed", "Couldn't load your blocked list.")}
						</span>
						<Button variant="outline" onClick={() => void blocks.refetch()}>
							{t("crew.retry", "Try again")}
						</Button>
					</div>
				)}
				{blocks.data?.length === 0 && (
					<p className="text-sm text-muted-foreground">
						{t("crew.blocks.empty", "You haven't blocked anyone.")}
					</p>
				)}
				<ul className="max-h-[60vh] overflow-y-auto">
					{blocks.data?.map((block) => {
						const person = directory(block.principal_id);
						return (
							<li
								key={block.principal_id}
								className="flex items-center gap-3 py-2"
							>
								<IdentityAvatar subject={block.principal_id} size={28} />
								<div className="min-w-0 flex-1">
									<p className="truncate text-sm font-medium">{person.name}</p>
									{block.kind === "bot" && (
										<p className="text-xs text-muted-foreground">
											{t("roomMembers.bot", "Bot")}
										</p>
									)}
								</div>
								<PrincipalBlockButton
									principal={{
										id: block.principal_id,
										name: person.name,
										kind: block.kind,
									}}
								/>
							</li>
						);
					})}
				</ul>
			</DialogContent>
		</Dialog>
	);
}
