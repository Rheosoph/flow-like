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
	speaker?: { id: string; name?: string | null } | null;
}

export interface BotFact {
	id: string;
	text: string;
	kind: string;
	subject: string;
	/** The entity the fact is about, when it has been resolved. */
	subject_id?: string | null;
	/** The entity at the other end of the fact, when it has one. */
	object_id?: string | null;
	/** How the subject relates to the object, e.g. "lives in". */
	relation?: string | null;
	source: string;
	confidence: number;
	recorded_at: string;
	valid_from: string | null;
	valid_to: string | null;
	geojson?: Record<string, unknown> | null;
	time?: string | null;
	expires_at?: string | null;
	superseded_by: string | null;
	evidence: BotFactEvidence[];
	/** Room positions cannot be opened in the owner's direct chat. */
	conversation_scope?: string | null;
}

export type BotEntityKind =
	| "person"
	| "place"
	| "organization"
	| "project"
	| "thing";

export interface BotEntity {
	id: string;
	name: string;
	kind: BotEntityKind | (string & {});
	/** The bot's owner. */
	owner: boolean;
	/** The participant account a group member's entity stands for. */
	participant?: string | null;
	aliases: string[];
	fact_count: number;
	created_at: string;
	updated_at: string;
	/** Entity ids are unique only within one scope; none is the direct chat. */
	conversation_scope?: string | null;
}

export interface BotEntityRef {
	id: string;
	scope?: string | null;
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

/** The people, places and things the facts are about, the owner first. */
export async function listBotEntities(
	owner: BotFactsOwner,
): Promise<BotEntity[]> {
	if (isTauri()) {
		return tauriInvoke<BotEntity[]>("bot_memory_entities", {
			appId: owner.appId,
		});
	}
	const response = await expectOk(
		await fetch(botUrl(owner.botId, "/entities"), {
			headers: headers(owner.token),
		}),
		"listing entities",
	);
	const body: unknown = await response.json();
	const entities = (body as { entities?: unknown } | null)?.entities;
	if (!Array.isArray(entities)) {
		throw new Error("listing entities returned no list");
	}
	return entities as BotEntity[];
}

function entityUrl(owner: BotFactsOwner, ref: BotEntityRef, suffix = "") {
	const scope = ref.scope ? `?scope=${encodeURIComponent(ref.scope)}` : "";
	return botUrl(
		owner.botId,
		`/entities/${encodeURIComponent(ref.id)}${suffix}${scope}`,
	);
}

/** Renames an entity or changes its kind; the old name stays one of its names. */
export async function updateBotEntity(
	owner: BotFactsOwner,
	ref: BotEntityRef,
	change: { name?: string; kind?: string },
): Promise<BotEntity> {
	if (isTauri()) {
		return tauriInvoke<BotEntity>("bot_memory_entity_update", {
			appId: owner.appId,
			entityId: ref.id,
			scope: ref.scope ?? null,
			name: change.name ?? null,
			kind: change.kind ?? null,
		});
	}
	const response = await expectOk(
		await fetch(entityUrl(owner, ref), {
			method: "PATCH",
			headers: headers(owner.token),
			body: JSON.stringify(change),
		}),
		"updating an entity",
	);
	return (await response.json()) as BotEntity;
}

/** Merges `ref` into another entity of the same scope. */
export async function mergeBotEntity(
	owner: BotFactsOwner,
	ref: BotEntityRef,
	into: string,
): Promise<BotEntity> {
	if (isTauri()) {
		return tauriInvoke<BotEntity>("bot_memory_entity_merge", {
			appId: owner.appId,
			entityId: ref.id,
			scope: ref.scope ?? null,
			into,
		});
	}
	const response = await expectOk(
		await fetch(entityUrl(owner, ref, "/merge"), {
			method: "POST",
			headers: headers(owner.token),
			body: JSON.stringify({ into }),
		}),
		"merging entities",
	);
	return (await response.json()) as BotEntity;
}

/** Forgets an entity with every fact about it; resolves to the forgotten facts. */
export async function forgetBotEntity(
	owner: BotFactsOwner,
	ref: BotEntityRef,
): Promise<BotFact[]> {
	if (isTauri()) {
		return tauriInvoke<BotFact[]>("bot_memory_entity_forget", {
			appId: owner.appId,
			entityId: ref.id,
			scope: ref.scope ?? null,
		});
	}
	const response = await expectOk(
		await fetch(entityUrl(owner, ref), {
			method: "DELETE",
			headers: headers(owner.token),
		}),
		"forgetting an entity",
	);
	const body: unknown = await response.json();
	const forgotten = (body as { forgotten?: unknown } | null)?.forgotten;
	return Array.isArray(forgotten) ? (forgotten as BotFact[]) : [];
}
