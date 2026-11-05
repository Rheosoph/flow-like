import type { HttpClient } from "./client.js";
import { FlowLikeError } from "./errors.js";
import { segment } from "./paths.js";
import type { JsonObject } from "./types.js";

export type ConversationKind = "direct" | "group";
export type ConversationEncryption = "plain" | "e2e";
export type InboxPrincipalKind = "user" | "bot";
export type InboxEntryKind =
	| "message"
	| "reaction"
	| "edit"
	| "delete"
	| "member";
export type SendableInboxEntryKind = Exclude<InboxEntryKind, "member">;

/** Envelope v1 the server can read. */
export interface PlainEnvelope<Body = JsonObject> {
	v: 1;
	enc: "none";
	body: Body;
}

/** Envelope v1 sealed to one member's inbox key; only that member can open it. */
export interface SealedEnvelope {
	v: 1;
	enc: string;
	key_id: string;
	enc_key: string;
	ct: string;
}

export type InboxEnvelope = PlainEnvelope | SealedEnvelope;

export interface InboxMessageBody {
	text: string;
	/** The `message_id` this message answers. */
	reply_to?: string;
}

export interface InboxReactionBody {
	target: string;
	emoji: string;
	op: "add" | "remove";
}

export interface InboxEditBody {
	target: string;
	text: string;
}

export interface InboxDeleteBody {
	target: string;
}

/** Written by the server when a member joins or leaves. */
export interface InboxMemberBody {
	op: "join" | "leave";
	principal_id: string;
	by: string;
}

export interface ConversationMember {
	/** A user sub or a bot id. */
	principal_id: string;
	kind: InboxPrincipalKind;
	/** Null for the person who started the conversation. */
	added_by: string | null;
	joined_at: string;
}

/** One principal's copy of one event. Every copy of the same event shares `message_id`. */
export interface InboxEntry {
	id: string;
	message_id: string;
	conversation_id: string;
	sender_id: string;
	kind: InboxEntryKind;
	envelope: InboxEnvelope;
	created_at: string;
	/** Opaque position; pass it back as `before`, `after` or `upTo`. */
	cursor: string;
}

export interface Conversation {
	id: string;
	kind: ConversationKind;
	title: string | null;
	encryption: ConversationEncryption;
	created_by: string;
	created_at: string;
	last_activity_at: string;
	/** Active members, in the order they joined. */
	members: ConversationMember[];
	/** Messages from others the reader has not read yet. */
	unread_count: number;
	/** The newest message in the reader's inbox. */
	last_entry: InboxEntry | null;
}

export interface CreateConversationRequest {
	kind: ConversationKind;
	/** Groups only; at most 120 characters. */
	title?: string | null;
	/** Everyone except you: your own bots and people who share an app with you. Direct takes exactly one. */
	members: string[];
	/** `plain` by default. */
	encryption?: ConversationEncryption;
}

export interface ReaderOptions {
	/** Read the inbox of one of your bots instead of your own (read-only). */
	as?: string;
}

export interface InboxEntriesQuery extends ReaderOptions {
	/** Only entries older than this cursor. */
	before?: string;
	/** Only entries newer than this cursor. */
	after?: string;
	/** 1 to 100; 50 by default. */
	limit?: number;
}

export interface InboxEntriesPage {
	/** Newest first. */
	entries: InboxEntry[];
	/**
	 * Pass as `before` for the next older page. With `after`, set when more entries lie between
	 * the oldest entry here and `after`: reload instead of keeping a gap.
	 */
	next_before: string | null;
}

export interface InboxHistoryOptions extends ReaderOptions {
	/** Start below this cursor instead of at the newest entry. */
	before?: string;
	/** Stop at this cursor (exclusive) instead of at the start of the conversation. */
	after?: string;
	/** Entries per request, 1 to 100. */
	pageSize?: number;
}

export interface SendInboxEntryRequest {
	/** 1 to 64 characters of `[A-Za-z0-9_-]`; generated when omitted. Resending an id returns the first result. */
	message_id?: string;
	kind: SendableInboxEntryKind;
	/** Plain conversations. */
	envelope?: PlainEnvelope<object>;
	/** End-to-end encrypted conversations: one sealed envelope per active member, keyed by principal id. */
	envelopes?: Record<string, SealedEnvelope>;
}

export interface SendOptions {
	/** Reuse an id to retry a send without sending twice; generated when omitted. */
	messageId?: string;
}

export interface SendMessageOptions extends SendOptions {
	/** The `message_id` this message answers. */
	replyTo?: string;
}

export interface ReactOptions extends SendOptions {
	/** `add` by default. */
	op?: "add" | "remove";
}

export interface InboxReadMarker {
	last_read_at: string;
}

export interface InboxKey {
	principal_id: string;
	key_id: string;
	algorithm: string;
	public_key: string;
	created_at: string;
}

/** Keys never change: register a new `key_id` to rotate. */
export interface RegisterInboxKeyRequest {
	/** Omit for your own key; the id of a bot you own registers that bot's key. */
	principal_id?: string;
	/** 1 to 128 characters of `[A-Za-z0-9._:-]`. */
	key_id: string;
	/** e.g. `hpke-x25519-sha256-chacha20poly1305`. */
	algorithm: string;
	public_key: string;
}

/** A fresh client-chosen message id: `msg_` and 32 hex digits. */
export function newInboxMessageId(): string {
	const random = globalThis.crypto;
	if (!random?.getRandomValues)
		throw new FlowLikeError(
			"Generating a message id needs Web Crypto (Node.js 19+); pass a message id instead",
		);
	const bytes = random.getRandomValues(new Uint8Array(16));
	return `msg_${Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
}

/** The plain envelope v1 around `body`. */
export function plainEnvelope<Body>(body: Body): PlainEnvelope<Body> {
	return { v: 1, enc: "none", body };
}

/** The body of an entry's plain envelope, e.g. `InboxMessageBody` for a message; undefined when sealed. */
export function plainBody<Body = JsonObject>(
	entry: Pick<InboxEntry, "envelope">,
): Body | undefined {
	const { envelope } = entry;
	return envelope.enc === "none" && "body" in envelope
		? (envelope.body as Body)
		: undefined;
}

/** The text of a plain message or edit; undefined for other kinds and sealed envelopes. */
export function inboxEntryText(
	entry: Pick<InboxEntry, "envelope">,
): string | undefined {
	const text = plainBody<{ text?: unknown }>(entry)?.text;
	return typeof text === "string" ? text : undefined;
}

const conversationPath = (conversationId: string) =>
	`/inbox/conversations/${segment(conversationId)}`;

/** Conversations between people and bots. These routes accept people only (PATs), not API keys. */
export function createInboxMethods(http: HttpClient) {
	const listInboxEntries = (
		conversationId: string,
		query: InboxEntriesQuery = {},
	): Promise<InboxEntriesPage> =>
		http.request("GET", `${conversationPath(conversationId)}/entries`, {
			query: {
				as: query.as,
				before: query.before,
				after: query.after,
				limit: query.limit,
			},
		});

	const sendInboxEntry = (
		conversationId: string,
		request: SendInboxEntryRequest,
	): Promise<InboxEntry> =>
		http.request("POST", `${conversationPath(conversationId)}/entries`, {
			body: {
				...request,
				message_id: request.message_id ?? newInboxMessageId(),
			},
		});

	const sendPlain = (
		conversationId: string,
		kind: SendableInboxEntryKind,
		body:
			| InboxMessageBody
			| InboxReactionBody
			| InboxEditBody
			| InboxDeleteBody,
		messageId?: string,
	) =>
		sendInboxEntry(conversationId, {
			message_id: messageId,
			kind,
			envelope: plainEnvelope(body),
		});

	return {
		/** Newest activity first, at most 200. */
		async listConversations(
			options: ReaderOptions = {},
		): Promise<Conversation[]> {
			return (
				await http.request<{ conversations: Conversation[] }>(
					"GET",
					"/inbox/conversations",
					{ query: { as: options.as } },
				)
			).conversations;
		},
		getConversation(
			conversationId: string,
			options: ReaderOptions = {},
		): Promise<Conversation> {
			return http.request("GET", conversationPath(conversationId), {
				query: { as: options.as },
			});
		},
		/** A direct conversation you already have with the member is returned instead of a new one. */
		createConversation(
			request: CreateConversationRequest,
		): Promise<Conversation> {
			return http.request("POST", "/inbox/conversations", { body: request });
		},
		/** Groups only; an empty or null title removes it. */
		renameConversation(
			conversationId: string,
			title: string | null,
		): Promise<Conversation> {
			return http.request("PATCH", conversationPath(conversationId), {
				body: { title },
			});
		},
		/** Current members are skipped. */
		addConversationMembers(
			conversationId: string,
			members: string[],
		): Promise<Conversation> {
			return http.request(
				"POST",
				`${conversationPath(conversationId)}/members`,
				{ body: { members } },
			);
		},
		/** Leave with your own id, remove one of your bots, or anyone from a group you started. */
		async removeConversationMember(
			conversationId: string,
			principalId: string,
		): Promise<void> {
			await http.request(
				"DELETE",
				`${conversationPath(conversationId)}/members/${segment(principalId)}`,
			);
		},
		/** One page of the reader's copy, newest first. */
		listInboxEntries,
		/** Every entry from `before` (or the newest) down to `after` (or the start), newest first, page by page. */
		async *iterateInboxEntries(
			conversationId: string,
			options: InboxHistoryOptions = {},
		): AsyncGenerator<InboxEntry, void, undefined> {
			let before = options.before;
			do {
				const page = await listInboxEntries(conversationId, {
					as: options.as,
					before,
					after: options.after,
					limit: options.pageSize,
				});
				yield* page.entries;
				before = page.next_before ?? undefined;
			} while (before);
		},
		/** Sends any event; returns your copy. Use `envelopes` in end-to-end encrypted conversations. */
		sendInboxEntry,
		sendInboxMessage(
			conversationId: string,
			text: string,
			options: SendMessageOptions = {},
		): Promise<InboxEntry> {
			return sendPlain(
				conversationId,
				"message",
				options.replyTo === undefined
					? { text }
					: { text, reply_to: options.replyTo },
				options.messageId,
			);
		},
		reactToInboxMessage(
			conversationId: string,
			targetMessageId: string,
			emoji: string,
			options: ReactOptions = {},
		): Promise<InboxEntry> {
			return sendPlain(
				conversationId,
				"reaction",
				{ target: targetMessageId, emoji, op: options.op ?? "add" },
				options.messageId,
			);
		},
		/** Only messages you sent. */
		editInboxMessage(
			conversationId: string,
			targetMessageId: string,
			text: string,
			options: SendOptions = {},
		): Promise<InboxEntry> {
			return sendPlain(
				conversationId,
				"edit",
				{ target: targetMessageId, text },
				options.messageId,
			);
		},
		/** Only messages you sent. */
		deleteInboxMessage(
			conversationId: string,
			targetMessageId: string,
			options: SendOptions = {},
		): Promise<InboxEntry> {
			return sendPlain(
				conversationId,
				"delete",
				{ target: targetMessageId },
				options.messageId,
			);
		},
		/** Moves your read marker to `upTo` (a cursor), or to now; it never moves back. */
		markConversationRead(
			conversationId: string,
			upTo?: string,
		): Promise<InboxReadMarker> {
			return http.request("POST", `${conversationPath(conversationId)}/read`, {
				body: upTo === undefined ? {} : { up_to: upTo },
			});
		},
		/** Active public keys of up to 50 people and bots. */
		async getInboxKeys(principalIds: string[]): Promise<InboxKey[]> {
			return (
				await http.request<{ keys: InboxKey[] }>("GET", "/inbox/keys", {
					query: { principals: principalIds.join(",") },
				})
			).keys;
		},
		registerInboxKey(request: RegisterInboxKeyRequest): Promise<InboxKey> {
			return http.request("PUT", "/inbox/keys", { body: request });
		},
		/** Revokes your key, or with `principalId` one of your bots' keys. */
		async revokeInboxKey(keyId: string, principalId?: string): Promise<void> {
			await http.request("DELETE", `/inbox/keys/${segment(keyId)}`, {
				query: { principal_id: principalId },
			});
		},
	};
}
