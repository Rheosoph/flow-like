"use client";

import { useTranslation } from "@flow-like/locales";
import CopyIcon from "lucide-react/dist/esm/icons/copy.js";
import ReplyIcon from "lucide-react/dist/esm/icons/reply.js";
import EditIcon from "lucide-react/dist/esm/icons/square-pen.js";
import type { ReactNode } from "react";
import { cn } from "../../../lib/utils";

const ACTION_CLASS =
	"inline-flex size-7 items-center justify-center rounded-md border bg-popover text-muted-foreground shadow-sm outline-none transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring disabled:opacity-50";

function RoomAction({
	label,
	onClick,
	disabled,
	children,
}: Readonly<{
	label: string;
	onClick: () => void;
	disabled?: boolean;
	children: ReactNode;
}>) {
	return (
		<button
			type="button"
			title={label}
			aria-label={label}
			disabled={disabled}
			onClick={onClick}
			className={ACTION_CLASS}
		>
			{children}
		</button>
	);
}

/**
 * A room message's actions as an overlay on its top corner: it reserves no height, shows on
 * hover or focus (see `data-fl-chat-room-actions` in global.css) and never moves the transcript.
 */
export function RoomMessageActions({
	align,
	disabled,
	onReply,
	onEdit,
	onCopy,
	menu,
}: Readonly<{
	align: "start" | "end";
	disabled?: boolean;
	onReply?: () => void;
	onEdit?: () => void;
	onCopy: () => void;
	menu?: ReactNode;
}>) {
	const { t } = useTranslation("chat");
	return (
		<div
			data-fl-chat-room-actions
			className={cn(
				"absolute -top-4 z-10 flex items-center gap-1",
				align === "end" ? "right-0" : "right-2",
			)}
		>
			{onReply && (
				<RoomAction
					label={t("replyToMessage", "Reply to message")}
					disabled={disabled}
					onClick={onReply}
				>
					<ReplyIcon className="size-4" />
				</RoomAction>
			)}
			{onEdit && (
				<RoomAction label={t("editMessage2", "Edit message")} onClick={onEdit}>
					<EditIcon className="size-4" />
				</RoomAction>
			)}
			<RoomAction label={t("copyMessage", "Copy message")} onClick={onCopy}>
				<CopyIcon className="size-4" />
			</RoomAction>
			{menu}
		</div>
	);
}
