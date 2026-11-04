"use client";

import { useTranslation } from "@flow-like/locales";
import {
	BellIcon,
	BellOffIcon,
	MoreHorizontalIcon,
	PinIcon,
	PinOffIcon,
	StarIcon,
} from "lucide-react";
import { toast } from "sonner";
import { useSetConversationMuted } from "../../hooks/use-inbox";
import { useSetConversationPinned } from "../../hooks/use-inbox-marks";
import { cn } from "../../lib/utils";
import type { IConversation } from "../../state/backend-state/inbox-state";
import { Button } from "../ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../ui/dropdown-menu";

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
 * What you can do with a conversation without opening it: pin it to the top of your list and,
 * with `withMute`, mute it. `onShowStarred` adds your starred messages of it.
 */
export function ConversationMenu({
	conversation,
	withMute = false,
	onShowStarred,
	className,
}: Readonly<{
	conversation: IConversation;
	withMute?: boolean;
	onShowStarred?: () => void;
	className?: string;
}>) {
	const { t } = useTranslation("bots");
	const pin = useSetConversationPinned();
	const mute = useSetConversationMuted();
	const failed = useFailedToast();
	const pinned = Boolean(conversation.pinned_at);
	const muted = Boolean(conversation.muted);
	const label = t("crew.conversationMenu.open", "Conversation options");
	return (
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
			</DropdownMenuContent>
		</DropdownMenu>
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
