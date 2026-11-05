import {
	CONVERSATION_FILE_APP_PREFIX,
	type IConversationFileReady,
	type IInboxAttachmentPart,
	type IInboxFileHandle,
	MAX_CONVERSATION_FILE_BYTES,
} from "../state/backend-state/inbox-state";

const UPLOAD_PARALLELISM = 3;

export function isConversationFileHandle(
	handle: Pick<IInboxFileHandle, "app_id">,
): boolean {
	return handle.app_id.startsWith(CONVERSATION_FILE_APP_PREFIX);
}

/** The id of a conversation file: its handle's path without the extension. */
export function conversationFileId(path: string): string {
	const dot = path.indexOf(".");
	return dot < 0 ? path : path.slice(0, dot);
}

export function conversationFileTooLarge(file: Pick<File, "name" | "size">) {
	return file.size > MAX_CONVERSATION_FILE_BYTES
		? `${file.name} is larger than ${MAX_CONVERSATION_FILE_BYTES / (1024 * 1024)} MB`
		: undefined;
}

/** PUTs the bytes to the signed upload URL the API minted. */
export async function putSignedFile(url: string, file: File): Promise<void> {
	const headers: Record<string, string> = {
		"Content-Type": file.type || "application/octet-stream",
	};
	if (url.includes(".blob.core.windows.net")) {
		headers["x-ms-blob-type"] = "BlockBlob";
	}
	const response = await fetch(url, { method: "PUT", body: file, headers });
	if (!response.ok) {
		throw new Error(
			`Uploading ${file.name} failed (${response.status}). Please try again.`,
		);
	}
}

/** The attachment parts for uploaded files, numbered after the `existing` attachments. */
export function attachmentParts(
	files: readonly IConversationFileReady[],
	existing: readonly IInboxAttachmentPart[] = [],
): IInboxAttachmentPart[] {
	return files.map((file, index) => ({
		kind: "attachment",
		id: `a${existing.length + index + 1}`,
		name: file.name,
		handle: {
			app_id: file.app_id,
			path: file.path,
			bytes: file.bytes,
			sha256: file.sha256,
			mime: file.mime,
		},
	}));
}

/** Runs `work` over `items` with a few in flight; the first failure rejects after the rest settle. */
export async function inBatches<T, R>(
	items: readonly T[],
	work: (item: T) => Promise<R>,
): Promise<R[]> {
	const results = new Array<R>(items.length);
	let next = 0;
	const lane = async () => {
		while (next < items.length) {
			const index = next++;
			results[index] = await work(items[index] as T);
		}
	};
	const settled = await Promise.allSettled(
		Array.from({ length: Math.min(UPLOAD_PARALLELISM, items.length) }, lane),
	);
	const failed = settled.find((result) => result.status === "rejected");
	if (failed?.status === "rejected") throw failed.reason;
	return results;
}
