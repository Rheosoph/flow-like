import type { IAttachment } from "../components/interfaces/chat-default/chat-db";
import type { IInboxAttachmentPart } from "../state/backend-state/inbox-state";
import { isWorkspaceHandle, workspaceFileUrl } from "./workspace-file-url";

export const isHttpUrl = (value: unknown): value is string =>
	typeof value === "string" && /^https?:\/\//i.test(value);

export const isConversationFilePart = (
	part: IInboxAttachmentPart,
): part is IInboxAttachmentPart & {
	handle: NonNullable<IInboxAttachmentPart["handle"]>;
} => Boolean(part.handle) && !isWorkspaceHandle(part.handle as never);

/**
 * A part the chat can render without a conversation-file request: a link, or a file in a bot
 * workspace (its `storage://` url is signed when it is rendered).
 */
export function directAttachmentOf(
	part: IInboxAttachmentPart,
): IAttachment | undefined {
	if (part.handle && isWorkspaceHandle(part.handle)) {
		return {
			url: workspaceFileUrl(part.handle),
			name: part.name,
			type: part.handle.mime,
			size: part.handle.bytes,
		};
	}
	if (!part.handle && isHttpUrl(part.url)) {
		return {
			url: part.url,
			name: part.name,
			...(part.mime ? { type: part.mime } : {}),
		};
	}
	return undefined;
}
