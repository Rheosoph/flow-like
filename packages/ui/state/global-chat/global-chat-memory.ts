// Profile-scoped assistant-memory operations, abstracted across platforms so the shared global-chat
// UI works on desktop and web. Desktop calls the Tauri commands (local LanceDB); web calls the
// server endpoints under `/api/v1/ai/global-chat/memory` (user-scoped storage). Mirrors the
// `use-copilot-sdk` pattern: `isTauri()` picks the path and Tauri is imported lazily.

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

function memoryUrl(profileId: string, suffix = ""): string {
	return `${getApiUrl(null, `ai/global-chat/memory${suffix}`)}?profile_id=${encodeURIComponent(profileId)}`;
}

/** Stored-memory count + the embedding model that produced it, for warning before a model switch. */
export async function globalChatMemoryStatus(
	profileId: string,
	token?: string,
): Promise<MemoryStatus> {
	if (isTauri()) {
		return tauriInvoke<MemoryStatus>("global_chat_memory_status", {
			profileId,
		});
	}
	const res = await fetch(memoryUrl(profileId), { headers: apiHeaders(token) });
	if (!res.ok) throw new Error(`memory status failed: ${res.status}`);
	return res.json();
}

/** Drop the whole memory table for a profile (used when the embedding model changes). */
export async function clearGlobalChatMemory(
	profileId: string,
	token?: string,
): Promise<void> {
	if (isTauri()) {
		await tauriInvoke("global_chat_clear_memory", { profileId });
		return;
	}
	const res = await fetch(memoryUrl(profileId), {
		method: "DELETE",
		headers: apiHeaders(token),
	});
	if (!res.ok) throw new Error(`clear memory failed: ${res.status}`);
}

/** List a profile's stored observations, newest first, for review/management in the UI. */
export async function listGlobalChatMemories(
	profileId: string,
	token?: string,
): Promise<MemoryEntry[]> {
	if (isTauri()) {
		return tauriInvoke<MemoryEntry[]>("global_chat_list_memories", {
			profileId,
		});
	}
	const res = await fetch(memoryUrl(profileId, "/entries"), {
		headers: apiHeaders(token),
	});
	if (!res.ok) throw new Error(`list memories failed: ${res.status}`);
	return res.json();
}

/** Delete a single stored observation by id. */
export async function deleteGlobalChatMemory(
	profileId: string,
	id: string,
	token?: string,
): Promise<void> {
	if (isTauri()) {
		await tauriInvoke("global_chat_delete_memory", { profileId, id });
		return;
	}
	const res = await fetch(
		`${getApiUrl(null, `ai/global-chat/memory/${encodeURIComponent(id)}`)}?profile_id=${encodeURIComponent(profileId)}`,
		{ method: "DELETE", headers: apiHeaders(token) },
	);
	if (!res.ok) throw new Error(`delete memory failed: ${res.status}`);
}
