// Assistant-memory operations, abstracted across platforms so the shared global-chat UI works on
// desktop and web. Flux remembers per profile; every bot keeps its own memory. Desktop calls the
// Tauri commands (local LanceDB); web calls the server endpoints under
// `/api/v1/ai/global-chat/memory` (user-scoped storage). Mirrors the `use-copilot-sdk` pattern:
// `isTauri()` picks the path and Tauri is imported lazily.

import { getApiUrl } from "../../lib/api-url";
import { isTauri } from "../../lib/platform";

export interface MemoryStatus {
	count: number;
	embedding_model_id: string | null;
}

export interface MemoryEntry {
	id: string;
	content: string;
	role: string;
	timestamp: number;
}

/** Whose memory: Flux's in a profile (`botId` null), or one bot's. */
export interface MemoryOwner {
	profileId: string;
	botId: string | null;
}

async function tauriInvoke<T>(
	command: string,
	args: Record<string, unknown>,
): Promise<T> {
	const { invoke } = await import("@tauri-apps/api/core");
	return invoke<T>(command, args);
}

function apiHeaders(token?: string): Record<string, string> {
	return {
		"content-type": "application/json",
		...(token ? { authorization: `Bearer ${token}` } : {}),
	};
}

function ownerArgs({ profileId, botId }: MemoryOwner) {
	return botId ? { profileId, botId } : { profileId };
}

function memoryUrl({ profileId, botId }: MemoryOwner, suffix = ""): string {
	const query = new URLSearchParams({ profile_id: profileId });
	if (botId) query.set("bot_id", botId);
	return `${getApiUrl(null, `ai/global-chat/memory${suffix}`)}?${query}`;
}

/** Stored-memory count + the embedding model that produced it, for warning before a model switch. */
export async function globalChatMemoryStatus(
	owner: MemoryOwner,
	token?: string,
): Promise<MemoryStatus> {
	if (isTauri()) {
		return tauriInvoke<MemoryStatus>(
			"global_chat_memory_status",
			ownerArgs(owner),
		);
	}
	const res = await fetch(memoryUrl(owner), { headers: apiHeaders(token) });
	if (!res.ok) throw new Error(`memory status failed: ${res.status}`);
	return res.json();
}

/** Drop the whole memory table of its owner (used when the embedding model changes). */
export async function clearGlobalChatMemory(
	owner: MemoryOwner,
	token?: string,
): Promise<void> {
	if (isTauri()) {
		await tauriInvoke("global_chat_clear_memory", ownerArgs(owner));
		return;
	}
	const res = await fetch(memoryUrl(owner), {
		method: "DELETE",
		headers: apiHeaders(token),
	});
	if (!res.ok) throw new Error(`clear memory failed: ${res.status}`);
}

/** List the owner's stored observations, newest first, for review/management in the UI. */
export async function listGlobalChatMemories(
	owner: MemoryOwner,
	token?: string,
): Promise<MemoryEntry[]> {
	if (isTauri()) {
		return tauriInvoke<MemoryEntry[]>(
			"global_chat_list_memories",
			ownerArgs(owner),
		);
	}
	const res = await fetch(memoryUrl(owner, "/entries"), {
		headers: apiHeaders(token),
	});
	if (!res.ok) throw new Error(`list memories failed: ${res.status}`);
	const entries: unknown = await res.json();
	if (!Array.isArray(entries)) {
		throw new Error(`list memories returned ${typeof entries}, not a list`);
	}
	return entries;
}

/** Delete a single stored observation by id. */
export async function deleteGlobalChatMemory(
	owner: MemoryOwner,
	id: string,
	token?: string,
): Promise<void> {
	if (isTauri()) {
		await tauriInvoke("global_chat_delete_memory", { ...ownerArgs(owner), id });
		return;
	}
	const res = await fetch(memoryUrl(owner, `/${encodeURIComponent(id)}`), {
		method: "DELETE",
		headers: apiHeaders(token),
	});
	if (!res.ok) throw new Error(`delete memory failed: ${res.status}`);
}
