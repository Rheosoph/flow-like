// The facts a bot remembers, for the memory tab: desktop reads the bot's workspace database
// through Tauri commands, the web through the hub's `/bots/{id}/memory` routes.

import { getApiUrl } from "../../lib/api-url";
import { isTauri } from "../../lib/platform";

export interface BotFactEvidence {
	/** The history position (`#seq`) the fact was learned from. */
	seq: number;
	message_id: string;
	quote: string;
	at: string;
}

export interface BotFact {
	id: string;
	text: string;
	kind: string;
	subject: string;
	source: string;
	confidence: number;
	recorded_at: string;
	valid_from: string | null;
	valid_to: string | null;
	superseded_by: string | null;
	evidence: BotFactEvidence[];
}

export interface BotFactsOwner {
	botId: string;
	/** The bot's workspace app, where the desktop finds its memory. */
	appId: string;
	token?: string;
}

async function tauriInvoke<T>(
	command: string,
	args: Record<string, unknown>,
): Promise<T> {
	const { invoke } = await import("@tauri-apps/api/core");
	return invoke<T>(command, args);
}

function headers(token?: string): Record<string, string> {
	return {
		"content-type": "application/json",
		...(token ? { authorization: `Bearer ${token}` } : {}),
	};
}

function botUrl(botId: string, suffix: string): string {
	return getApiUrl(null, `bots/${encodeURIComponent(botId)}/memory${suffix}`);
}

async function expectOk(response: Response, action: string): Promise<Response> {
	if (!response.ok) throw new Error(`${action} failed: ${response.status}`);
	return response;
}

/** What the bot remembers, newest first. */
export async function listBotFacts(owner: BotFactsOwner): Promise<BotFact[]> {
	if (isTauri()) {
		return tauriInvoke<BotFact[]>("bot_memory_facts", { appId: owner.appId });
	}
	const response = await expectOk(
		await fetch(botUrl(owner.botId, "/facts"), {
			headers: headers(owner.token),
		}),
		"listing facts",
	);
	const body: unknown = await response.json();
	const facts = (body as { facts?: unknown } | null)?.facts;
	if (!Array.isArray(facts)) {
		throw new Error("listing facts returned no list");
	}
	return facts as BotFact[];
}

/** Hard-deletes one fact, the way the `memory_forget` tool does; the chat keeps what was said. */
export async function forgetBotFact(
	owner: BotFactsOwner,
	factId: string,
): Promise<void> {
	if (isTauri()) {
		await tauriInvoke("bot_memory_forget", { appId: owner.appId, factId });
		return;
	}
	await expectOk(
		await fetch(botUrl(owner.botId, `/facts/${encodeURIComponent(factId)}`), {
			method: "DELETE",
			headers: headers(owner.token),
		}),
		"forgetting a fact",
	);
}

/** Forgets everything the bot remembered; its chat history stays. */
export async function resetBotMemory(owner: BotFactsOwner): Promise<void> {
	if (isTauri()) {
		await tauriInvoke("bot_memory_reset", {
			appId: owner.appId,
			botId: owner.botId,
		});
		return;
	}
	await expectOk(
		await fetch(botUrl(owner.botId, ""), {
			method: "DELETE",
			headers: headers(owner.token),
		}),
		"resetting memory",
	);
}
