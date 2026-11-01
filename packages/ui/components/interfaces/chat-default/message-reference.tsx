"use client";

import { useTranslation } from "@flow-like/locales";
import ReplyIcon from "lucide-react/dist/esm/icons/reply.js";
import XIcon from "lucide-react/dist/esm/icons/x.js";
import type { IChatMessageReference } from "./chat-message-interaction";

export function MessageReference({
	reference,
	onCancel,
	label,
}: {
	reference: IChatMessageReference;
	onCancel?: () => void;
	label?: string;
}) {
	const { t } = useTranslation("chat");
	return (
		<blockquote
			className="mb-2 flex min-w-0 gap-2 rounded-md border-l-2 border-primary/50 bg-muted/40 px-3 py-2 text-sm"
			data-fl-chat-reference={reference.messageId}
		>
			<ReplyIcon className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />
			<div className="min-w-0 flex-1">
				<div className="mb-0.5 text-xs font-medium text-muted-foreground">
					{label ??
						(reference.role === "assistant"
							? t("replyingToAssistant", "Replying to assistant")
							: t("replyingToMessage", "Replying to message"))}
				</div>
				<p className="line-clamp-3 whitespace-pre-wrap break-words">
					{reference.text || t("messageWithoutText", "Message without text")}
				</p>
			</div>
			{onCancel && (
				<button
					type="button"
					className="self-start rounded p-1 text-muted-foreground hover:bg-muted hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
					aria-label={t("cancelReply", "Cancel reply")}
					onClick={onCancel}
				>
					<XIcon className="size-4" />
				</button>
			)}
		</blockquote>
	);
}
