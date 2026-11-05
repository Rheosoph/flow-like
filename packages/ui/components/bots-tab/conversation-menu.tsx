"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BanIcon,
	BellIcon,
	BellOffIcon,
	LogOutIcon,
	MoreHorizontalIcon,
	PencilIcon,
	PinIcon,
	PinOffIcon,
	StarIcon,
} from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { useSetConversationMuted } from "../../hooks/use-inbox";
import { useInboxBlocks } from "../../hooks/use-inbox-blocks";
import { useSetConversationPinned } from "../../hooks/use-inbox-marks";
import { cn } from "../../lib/utils";
import type { IConversation } from "../../state/backend-state/inbox-state";
import { Button } from "../ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../ui/dropdown-menu";
import { isSelfPrincipal } from "./crew-model";
import { LeaveRoomDialog } from "./leave-room-dialog";
import {
	type BlockTarget,
	PrincipalBlockDialog,
	principalBlock,
} from "./principal-block-action";
import { RoomSettingsDialog } from "./room-settings-dialog";

/** Mute applies to chats with other people; a chat with only your bots never pushes. */
export function canMuteConversation(conversation: IConversation): boolean {
	return (
		!conversation.local &&
		conversation.members.filter((member) => member.kind === "user").length > 1
	);
}

function useFailedToast() {
	const { t } = useTranslation("bots");
	return (error: unknown) => {
		console.warn("Updating the conversation failed", error);
		toast.error(
			t("crew.messages.actionFailed", "That didn't go through. Try again."),
		);
	};
}

/**
 * What you can do with a conversation without opening it: pin it to the top of your list, rename
 * a group and change its picture and, with `withMute`, mute it. `onShowStarred` adds your starred
 * messages of it.
 */
export function ConversationMenu({
	conversation,
	defaultTitle,
	withMute = false,
	onShowStarred,
	me,
	onLeave,
	blockTarget,
	className,
}: Readonly<{
	conversation: IConversation;
	/** What a group is called while it has no name. */
	defaultTitle?: string;
	withMute?: boolean;
	onShowStarred?: () => void;
	me?: string | null;
	onLeave?: () => void;
	blockTarget?: BlockTarget;
	className?: string;
}>) {
	const { t } = useTranslation("bots");
	const pin = useSetConversationPinned();
	const mute = useSetConversationMuted();
	const failed = useFailedToast();
	const [editing, setEditing] = useState(false);
	const [leaving, setLeaving] = useState(false);
	const [blocking, setBlocking] = useState(false);
	const blocks = useInboxBlocks(Boolean(blockTarget));
	const existingBlock = principalBlock(blocks.data, blockTarget);
	const inheritedBlock =
		existingBlock && existingBlock.principal_id !== blockTarget?.id;
	const pinned = Boolean(conversation.pinned_at);
	const muted = Boolean(conversation.muted);
	const group = conversation.kind === "group";
	const self = conversation.members.find(
		(member) =>
			member.kind === "user" && isSelfPrincipal(member.principal_id, me),
	);
	const label = t("crew.conversationMenu.open", "Conversation options");
	return (
		<>
			<DropdownMenu>
				<DropdownMenuTrigger asChild>
					<Button
						type="button"
						variant="ghost"
						size="icon"
						className={cn("shrink-0 text-muted-foreground", className)}
						aria-label={label}
						title={label}
					>
						<MoreHorizontalIcon aria-hidden />
					</Button>
				</DropdownMenuTrigger>
				<DropdownMenuContent align="end">
					<DropdownMenuItem
						onSelect={() =>
							pin.mutate(
								{ conversationId: conversation.id, pinned: !pinned },
								{ onError: failed },
							)
						}
					>
						{pinned ? <PinOffIcon aria-hidden /> : <PinIcon aria-hidden />}
						{pinned
							? t("crew.marks.unpinConversation", "Unpin from top")
							: t("crew.marks.pinConversation", "Pin to top")}
					</DropdownMenuItem>
					{withMute && canMuteConversation(conversation) && (
						<DropdownMenuItem
							onSelect={() =>
								mute.mutate(
									{ conversationId: conversation.id, muted: !muted },
									{ onError: failed },
								)
							}
						>
							{muted ? <BellIcon aria-hidden /> : <BellOffIcon aria-hidden />}
							{muted
								? t("crew.mute.unmute", "Unmute notifications")
								: t("crew.mute.mute", "Mute notifications")}
						</DropdownMenuItem>
					)}
					{onShowStarred && (
						<DropdownMenuItem onSelect={onShowStarred}>
							<StarIcon aria-hidden />
							{t("crew.marks.starredTitle", "Starred messages")}
						</DropdownMenuItem>
					)}
					{group && (
						<DropdownMenuItem onSelect={() => setEditing(true)}>
							<PencilIcon aria-hidden />
							{t("crew.roomSettings.open", "Edit name and picture")}
						</DropdownMenuItem>
					)}
					{blockTarget && (
						<DropdownMenuItem
							disabled={!blocks.data || Boolean(inheritedBlock)}
							onSelect={() => setBlocking(true)}
						>
							<BanIcon aria-hidden />
							{inheritedBlock
								? t("crew.blocks.ownerBlocked", "Owner blocked")
								: existingBlock
									? t("crew.blocks.unblock", "Unblock")
									: blockTarget.kind === "bot"
										? t("crew.blocks.blockBot", "Block bot")
										: t("crew.blocks.blockUser", "Block user")}
						</DropdownMenuItem>
					)}
					{group && self && (
						<>
							<DropdownMenuSeparator />
							<DropdownMenuItem
								className="text-destructive focus:text-destructive"
								onSelect={() => setLeaving(true)}
							>
								<LogOutIcon aria-hidden />
								{t("crew.leaveRoom.action", "Leave room")}
							</DropdownMenuItem>
						</>
					)}
				</DropdownMenuContent>
			</DropdownMenu>
			{blocking && blockTarget && (
				<PrincipalBlockDialog
					principal={blockTarget}
					blocked={Boolean(existingBlock)}
					onOpenChange={setBlocking}
				/>
			)}
			{group && (
				<RoomSettingsDialog
					open={editing}
					onOpenChange={setEditing}
					conversation={conversation}
					defaultTitle={defaultTitle}
				/>
			)}
			{group && self && leaving && (
				<LeaveRoomDialog
					open
					onOpenChange={setLeaving}
					conversationId={conversation.id}
					principalId={self.principal_id}
					onLeave={onLeave}
				/>
			)}
		</>
	);
}

/** The mark a pinned conversation carries in lists. */
export function PinnedMark({ className }: Readonly<{ className?: string }>) {
	const { t } = useTranslation("bots");
	const label = t("crew.marks.pinnedConversation", "Pinned");
	return (
		<PinIcon
			className={cn("size-3 shrink-0 text-muted-foreground", className)}
			role="img"
			aria-label={label}
		/>
	);
}
