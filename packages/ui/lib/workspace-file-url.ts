import type { IInboxFileHandle } from "../state/backend-state/inbox-state";

const STORAGE_FILE_URL = /^storage:\/\/apps\/([^/]+)\/app\/(.+)$/;

const encodePath = (path: string) =>
	path
		.split("/")
		.map((segment) => encodeURIComponent(segment))
		.join("/");

/** Mirrors `flow_like_bot_tools::files::markdown_url`. */
export function workspaceFileUrl(
	handle: Pick<IInboxFileHandle, "app_id" | "path">,
): string {
	return `storage://apps/${encodeURIComponent(handle.app_id)}/app/${encodePath(handle.path)}`;
}

export function parseWorkspaceFileUrl(
	url: string,
): { appId: string; path: string } | undefined {
	const match = STORAGE_FILE_URL.exec(url);
	if (!match) return undefined;
	try {
		return {
			appId: decodeURIComponent(match[1]),
			path: match[2].split("/").map(decodeURIComponent).join("/"),
		};
	} catch {
		return undefined;
	}
}

export const isWorkspaceHandle = (handle: Pick<IInboxFileHandle, "app_id">) =>
	!handle.app_id.startsWith("conversation_");
