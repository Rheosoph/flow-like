"use client";

import { useTranslation } from "@flow-like/locales";
import { ImageIcon } from "lucide-react";
import { useMemo } from "react";
import { workspaceFileUrl } from "../../lib/workspace-file-url";
import type { IInboxFileHandle } from "../../state/backend-state/inbox-state";
import type { ProcessedAttachment } from "../interfaces/chat-default/attachment";
import { useResolvedAttachments } from "../interfaces/chat-default/hooks/use-resolved-attachments";

/** The picture a bot asks to show in a room, as its members would see it. */
export function ApprovalPicture({
	picture,
	room,
}: Readonly<{ picture: IInboxFileHandle; room?: string }>) {
	const { t } = useTranslation("bots");
	const files = useMemo<ProcessedAttachment[]>(() => {
		const name = picture.path.split("/").pop() || picture.path;
		return [
			{
				url: workspaceFileUrl(picture),
				name,
				displayName: name,
				ext: name.includes(".") ? (name.split(".").pop() ?? "") : "",
				type: "image",
				isDataUrl: false,
				size: picture.bytes,
			},
		];
	}, [picture]);
	const [shown] = useResolvedAttachments(files);
	const src = shown?.access ? undefined : shown?.url;
	const alt = t(
		"crew.approval.picture.alt",
		"The picture the bot wants to show",
	);
	return (
		<figure data-slot="approval-picture" className="space-y-1.5">
			{src ? (
				<img
					src={src}
					alt={alt}
					className="max-h-72 w-full rounded-md border bg-muted object-contain"
				/>
			) : (
				<div
					role="img"
					aria-label={alt}
					aria-busy={shown?.access === "pending"}
					className="flex h-32 flex-col items-center justify-center gap-1 rounded-md border bg-muted text-xs text-muted-foreground"
				>
					<ImageIcon className="size-5" aria-hidden />
					{shown?.access === "unavailable" &&
						t(
							"crew.approval.picture.unavailable",
							"The picture could not be loaded.",
						)}
				</div>
			)}
			<figcaption className="text-xs text-muted-foreground">
				{room
					? t(
							"crew.approval.picture.nextInRoom",
							"Everyone in {{room}} sees this picture once you allow it. Asked every time.",
							{ room },
						)
					: t(
							"crew.approval.picture.next",
							"Everyone in this group chat sees this picture once you allow it. Asked every time.",
						)}
			</figcaption>
		</figure>
	);
}
