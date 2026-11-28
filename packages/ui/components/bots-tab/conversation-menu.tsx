"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BanIcon,
	BellIcon,
	BellOffIcon,
	LogOutIcon,
	PencilIcon,
	PinIcon,
	PinOffIcon,
	StarIcon,
	Trash2Icon,
	UsersIcon,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { toast } from "sonner";
import { useSetConversationMuted } from "../../hooks/use-inbox";
import { useInboxBlocks } from "../../hooks/use-inbox-blocks";
import { useSetConversationPinned } from "../../hooks/use-inbox-marks";
import { cn } from "../../lib/utils";
import type { IConversation } from "../../state/backend-state/inbox-state";
import { isSelfPrincipal } from "./crew-model";
import { DeleteConversationDialog } from "./delete-conversation-dialog";
import { LeaveRoomDialog } from "./leave-room-dialog";
import {
	type BlockTarget,
	PrincipalBlockDialog,
	principalBlock,
} from "./principal-block-action";
import { RoomSettingsDialog } from "./room-settings-dialog";
import { type MenuItems, type MenuKit, OptionsDropdown } from "./row-menu";

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

export interface ConversationMenuOptions {
	conversation: IConversation;
	/** What a group is called while it has no name. */
	defaultTitle?: string;
	withMute?: boolean;
	onShowStarred?: () => void;
	/** Adds the room's members and invitations, for headers too narrow for their own button. */
	onShowMembers?: () => void;
	me?: string | null;
	/** Closes the conversation after leaving or deleting it. */
	onLeave?: () => void;
	blockTarget?: BlockTarget;
}

/**
 * What you can do with a conversation without opening it: pin it to the top of your list, rename
 * a group and change its picture and, with `withMute`, mute it. `onShowStarred` adds your starred
 * messages of it. `items` renders into a dropdown or a context menu; `dialogs` must stay mounted.
 */
export function useConversationMenu({
	conversation,
	defaultTitle,
	withMute = false,
	onShowStarred,
	onShowMembers,
	me,
	onLeave,
	blockTarget,
}: Readonly<ConversationMenuOptions>): {
	items: MenuItems;
	dialogs: ReactNode;
} {
	const { t } = useTranslation("bots");
	const pin = useSetConversationPinned();
	const mute = useSetConversationMuted();
	const failed = useFailedToast();
	const [editing, setEditing] = useState(false);
	const [leaving, setLeaving] = useState(false);
	const [deleting, setDeleting] = useState(false);
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
	const items = ({ Item, Separator }: MenuKit) => (
		<>
			<Item
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
			</Item>
			{withMute && canMuteConversation(conversation) && (
				<Item
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
				</Item>
			)}
			{onShowMembers && (
				<Item onSelect={onShowMembers}>
					<UsersIcon aria-hidden />
					{t("roomMembers.open", "Members and invitations")}
				</Item>
			)}
			{onShowStarred && (
				<Item onSelect={onShowStarred}>
					<StarIcon aria-hidden />
					{t("crew.marks.starredTitle", "Starred messages")}
				</Item>
			)}
			{group && (
				<Item onSelect={() => setEditing(true)}>
					<PencilIcon aria-hidden />
					{t("crew.roomSettings.open", "Edit name and picture")}
				</Item>
			)}
			{blockTarget && (
				<Item
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
				</Item>
			)}
			{group && self && (
				<>
					<Separator />
					<Item
						className="text-destructive focus:text-destructive"
						onSelect={() => setLeaving(true)}
					>
						<LogOutIcon aria-hidden />
						{t("crew.leaveRoom.action", "Leave room")}
					</Item>
				</>
			)}
			{self && (
				<>
					{!group && <Separator />}
					<Item
						className="text-destructive focus:text-destructive"
						onSelect={() => setDeleting(true)}
					>
						<Trash2Icon aria-hidden />
						{group
							? t("crew.deleteConversation.groupAction", "Leave and delete")
							: t("crew.deleteConversation.action", "Delete conversation")}
					</Item>
				</>
			)}
		</>
	);
	const dialogs = (
		<>
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
			{self && deleting && (
				<DeleteConversationDialog
					open
					onOpenChange={setDeleting}
					conversationId={conversation.id}
					kind={conversation.kind}
					onDelete={onLeave}
				/>
			)}
		</>
	);
	return { items, dialogs };
}

/** The conversation's options behind a "…" button. */
export function ConversationMenu({
	className,
	...options
}: Readonly<ConversationMenuOptions & { className?: string }>) {
	const { items, dialogs } = useConversationMenu(options);
	return (
		<>
			<OptionsDropdown items={items} className={className} />
			{dialogs}
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
