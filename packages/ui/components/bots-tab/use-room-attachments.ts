"use client";

import { useQueries } from "@tanstack/react-query";
import { useMemo } from "react";
import { inboxKeys } from "../../hooks/use-inbox";
import { conversationFileId } from "../../lib/conversation-files";
import { useBackend } from "../../state/backend-state";
import type { IInboxAttachmentPart } from "../../state/backend-state/inbox-state";
import type { IAttachment } from "../interfaces/chat-default/chat-db";
import type { ConversationMessage } from "./fold-entries";

/**
 * The attachments of a room's messages as the chat renders files: signed links, fetched once per
 * file and shared with `useConversationFileUrl`. A file shows as soon as its link arrives.
 */
export function useRoomAttachments(
	conversationId: string,
	messages: readonly ConversationMessage[],
): ReadonlyMap<string, IAttachment[]> {
	const state = useBackend().inboxState;
	const parts = useMemo(
		() =>
			messages.flatMap((message) =>
				(message.attachments ?? []).map((part) => ({
					messageId: message.id,
					part,
				})),
			),
		[messages],
	);
	const links = useQueries({
		queries: parts.map(({ part }) => ({
			queryKey: [
				...inboxKeys.all,
				"file",
				conversationId,
				conversationFileId(part.handle.path),
				"me",
			],
			queryFn: () =>
				(state as NonNullable<typeof state>).conversationFileDownload(
					conversationId,
					part.handle,
				),
			enabled: Boolean(state),
			staleTime: 4 * 60_000,
		})),
	});
	const urlKey = links.map((link) => link.data?.url ?? "").join("\n");
	return useMemo(() => {
		const urls = urlKey.split("\n");
		const byMessage = new Map<string, IAttachment[]>();
		parts.forEach(({ messageId, part }, index) => {
			const url = urls[index];
			if (url) {
				const list = byMessage.get(messageId) ?? [];
				list.push(attachmentOf(part, url));
				byMessage.set(messageId, list);
			}
		});
		return byMessage;
	}, [parts, urlKey]);
}

function attachmentOf(part: IInboxAttachmentPart, url: string): IAttachment {
	return {
		url,
		name: part.name,
		type: part.handle.mime,
		size: part.handle.bytes,
	};
}
