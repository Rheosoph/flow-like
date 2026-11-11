import type {
	IConversation,
	IInboxEntriesPage,
	IInboxEntry,
	IInboxEnvelope,
	IInboxKey,
	IInboxPlainEnvelope,
	IInboxSealedEnvelope,
	IInboxState,
	IPinnedMessage,
	ISendInboxEntryRequest,
	IStarredMessage,
	IStarredMessagesPage,
} from "../state/backend-state/inbox-state";

/** The key directory's `algorithm` and the sealed envelope's `enc`. */
export const INBOX_KEY_ALGORITHM = "hpke-x25519-sha256-chacha20poly1305";
/** Device keys one member's envelope is sealed to: the server's active key limit. */
const MAX_MEMBER_KEYS = 20;
const OPENED_LIMIT = 4_000;
const STALE_SEND_CODES = new Set([
	"INBOX_ENCRYPTION_MISMATCH",
	"INBOX_ENVELOPES_MISMATCH",
]);

export interface IInboxPublicKey {
	key_id: string;
	algorithm: string;
	public_key: string;
	fingerprint: string;
}

export interface IInboxSealScope {
	conversation_id: string;
	message_id: string;
	sender_id: string;
}

/** A member and the published keys of their devices. */
export interface IInboxSealMember {
	principal_id: string;
	keys: { key_id: string; public_key: string }[];
}

export interface IInboxSealedItem extends IInboxSealScope {
	envelope: IInboxEnvelope;
}

/** Holds this device's inbox key and does the sealing; the private key never leaves it. */
export interface IInboxCrypto {
	/** This device's active key for the person, made on first use. */
	key(principalId: string): Promise<IInboxPublicKey>;
	/** One envelope per member, sealed to each of their keys. */
	seal(
		scope: IInboxSealScope,
		members: IInboxSealMember[],
		body: unknown,
	): Promise<Record<string, IInboxSealedEnvelope>>;
	/** The bodies of `entries`; `null` where none of this device's keys opens one. */
	open(principalId: string, entries: IInboxSealedItem[]): Promise<unknown[]>;
}

/** Members of an encrypted conversation who have no device key yet, so nothing can reach them. */
export class InboxKeyMissingError extends Error {
	readonly code = "INBOX_KEY_MISSING";

	constructor(readonly principals: readonly string[]) {
		super(
			`Could not send an end-to-end encrypted message: ${principals.join(", ")} published no inbox key yet. A person publishes one by opening the app signed in; a bot's host publishes it while it runs.`,
		);
		this.name = "InboxKeyMissingError";
	}
}

export function isSealedEnvelope(
	envelope: IInboxEnvelope | null | undefined,
): envelope is IInboxSealedEnvelope {
	return Boolean(envelope && envelope.enc !== "none");
}

function plainEnvelope(body: unknown): IInboxPlainEnvelope {
	return { v: 1, enc: "none", body };
}

function errorCode(error: unknown): string | undefined {
	const code = (error as { code?: unknown } | null)?.code;
	return typeof code === "string" ? code : undefined;
}

/** Published keys grouped by member, newest last; members without a key are left out. */
export function sealMembers(
	memberIds: readonly string[],
	keys: readonly IInboxKey[],
): Map<string, IInboxSealMember> {
	const members = new Map<string, IInboxSealMember>();
	for (const key of keys) {
		if (key.algorithm !== INBOX_KEY_ALGORITHM) continue;
		if (!memberIds.includes(key.principal_id)) continue;
		const member = members.get(key.principal_id) ?? {
			principal_id: key.principal_id,
			keys: [],
		};
		if (!member.keys.some((known) => known.key_id === key.key_id)) {
			member.keys.push({ key_id: key.key_id, public_key: key.public_key });
		}
		members.set(key.principal_id, member);
	}
	for (const member of members.values()) {
		member.keys = member.keys.slice(-MAX_MEMBER_KEYS);
	}
	return members;
}

type ConversationShape = Pick<IConversation, "id" | "encryption" | "members">;

/**
 * Seals what the person sends to an end-to-end encrypted conversation and opens what this
 * device's keys can open, so every reader of the inbox sees plain envelopes. Entries sealed to
 * keys this device doesn't hold stay sealed.
 */
export class InboxEndToEnd {
	private readonly opened = new Map<string, unknown>();
	private readonly conversations = new Map<string, ConversationShape>();
	private registration?: { sub: string; done: Promise<void> };

	constructor(
		private readonly inbox: IInboxState,
		private readonly crypto: IInboxCrypto,
		private readonly reader: () => string | undefined,
	) {}

	/** Publishes this device's key once per signed-in person, so members can seal to it. */
	register(): Promise<void> {
		const sub = this.reader();
		if (!sub) {
			return Promise.reject(
				new Error(
					"Could not use end-to-end encryption: it needs an account; sign in first",
				),
			);
		}
		if (this.registration?.sub === sub) return this.registration.done;
		const done = this.crypto.key(sub).then(async (key) => {
			await this.inbox.registerKey({
				key_id: key.key_id,
				algorithm: key.algorithm,
				public_key: key.public_key,
			});
		});
		const registration = { sub, done };
		this.registration = registration;
		done.catch(() => {
			if (this.registration === registration) this.registration = undefined;
		});
		return done;
	}

	remember(conversation: IConversation, requestedId = conversation.id): void {
		const shape = {
			id: conversation.id,
			encryption: conversation.encryption,
			members: conversation.members,
		};
		this.conversations.set(requestedId, shape);
		this.conversations.set(conversation.id, shape);
		if (conversation.encryption === "e2e") {
			this.register().catch((error) =>
				console.warn(
					"[Inbox] Publishing this device's inbox key failed",
					error,
				),
			);
		}
	}

	private async conversation(
		conversationId: string,
		fresh: boolean,
	): Promise<ConversationShape> {
		const known = fresh ? undefined : this.conversations.get(conversationId);
		if (known) return known;
		const conversation = await this.inbox.getConversation(conversationId);
		this.remember(conversation, conversationId);
		return conversation;
	}

	private cache(entryId: string, body: unknown): void {
		if (this.opened.size >= OPENED_LIMIT) {
			const oldest = this.opened.keys().next().value;
			if (oldest !== undefined) this.opened.delete(oldest);
		}
		this.opened.set(entryId, body);
	}

	/**
	 * `request` as the conversation takes it: sealed to every member's keys when encrypted. A
	 * call entry says only that a call started or ended and the server rings people from it, so
	 * it stays plain, like the joins and renames the server writes there.
	 */
	async prepare(
		conversationId: string,
		request: ISendInboxEntryRequest,
		fresh = false,
	): Promise<ISendInboxEntryRequest> {
		if (!request.envelope || request.envelopes) return request;
		if (request.kind === "call") return request;
		const conversation = await this.conversation(conversationId, fresh);
		if (conversation.encryption !== "e2e") return request;
		const sub = this.reader();
		if (!sub)
			throw new Error("Could not send: encrypted messages need an account");
		await this.register();
		const memberIds = conversation.members.map((member) => member.principal_id);
		const members = sealMembers(memberIds, await this.inbox.getKeys(memberIds));
		const missing = memberIds.filter((id) => !members.has(id));
		if (missing.length > 0) throw new InboxKeyMissingError(missing);
		const envelopes = await this.crypto.seal(
			{
				conversation_id: conversation.id,
				message_id: request.message_id,
				sender_id: sub,
			},
			[...members.values()],
			request.envelope.body,
		);
		return {
			message_id: request.message_id,
			kind: request.kind,
			envelopes,
		};
	}

	async send(
		conversationId: string,
		request: ISendInboxEntryRequest,
	): Promise<IInboxEntry> {
		let entry: IInboxEntry;
		try {
			entry = await this.inbox.sendEntry(
				conversationId,
				await this.prepare(conversationId, request),
			);
		} catch (error) {
			if (!STALE_SEND_CODES.has(errorCode(error) ?? "")) throw error;
			entry = await this.inbox.sendEntry(
				conversationId,
				await this.prepare(conversationId, request, true),
			);
		}
		if (request.envelope && isSealedEnvelope(entry.envelope)) {
			this.cache(entry.id, request.envelope.body);
			return { ...entry, envelope: plainEnvelope(request.envelope.body) };
		}
		return entry;
	}

	/** `entries` with every envelope this device can open replaced by its plain form. */
	async open(entries: IInboxEntry[], as?: string): Promise<IInboxEntry[]> {
		const sub = this.reader();
		if (!sub || (as && as !== sub)) return entries;
		const pending = entries.filter(
			(entry) => isSealedEnvelope(entry.envelope) && !this.opened.has(entry.id),
		);
		if (pending.length > 0) {
			try {
				const bodies = await this.crypto.open(
					sub,
					pending.map((entry) => ({
						conversation_id: entry.conversation_id,
						message_id: entry.message_id,
						sender_id: entry.sender_id,
						envelope: entry.envelope,
					})),
				);
				pending.forEach((entry, index) => {
					this.cache(entry.id, bodies[index] ?? null);
				});
			} catch (error) {
				console.warn("[Inbox] Opening encrypted messages failed", error);
			}
		}
		return entries.map((entry) => {
			const body = this.opened.get(entry.id);
			return body === undefined || body === null
				? entry
				: { ...entry, envelope: plainEnvelope(body) };
		});
	}

	/** Opens the sealed ones among `entries` in one call and finds each by id. */
	private async opener(
		entries: readonly (IInboxEntry | null)[],
		as?: string,
	): Promise<<E extends IInboxEntry | null>(entry: E) => E> {
		const present = entries.filter((entry): entry is IInboxEntry =>
			Boolean(entry),
		);
		if (!present.some((entry) => isSealedEnvelope(entry.envelope))) {
			return (entry) => entry;
		}
		const opened = new Map(
			(await this.open(present, as)).map((entry) => [entry.id, entry]),
		);
		return <E extends IInboxEntry | null>(entry: E) =>
			(entry ? (opened.get(entry.id) ?? entry) : entry) as E;
	}

	async openConversations(
		conversations: IConversation[],
		as?: string,
	): Promise<IConversation[]> {
		const swap = await this.opener(
			conversations.map((conversation) => conversation.last_entry),
			as,
		);
		return conversations.map((conversation) => ({
			...conversation,
			last_entry: swap(conversation.last_entry),
		}));
	}

	async openMarks<T extends IPinnedMessage | IStarredMessage>(
		marks: T[],
		as?: string,
	): Promise<T[]> {
		const swap = await this.opener(
			marks.flatMap((mark) => [mark.entry, mark.latest_edit]),
			as,
		);
		return marks.map((mark) => ({
			...mark,
			entry: swap(mark.entry),
			latest_edit: swap(mark.latest_edit),
		}));
	}
}

/**
 * `inbox` with end-to-end encryption: sends to encrypted conversations are sealed, and every
 * read hands back entries this device can open in their plain form.
 */
export function withEndToEnd(
	inbox: IInboxState,
	crypto: IInboxCrypto,
	reader: () => string | undefined,
): IInboxState {
	const e2e = new InboxEndToEnd(inbox, crypto, reader);
	const overrides: Partial<IInboxState> = {
		listConversations: async (as) => {
			const conversations = await inbox.listConversations(as);
			if (!as) {
				for (const conversation of conversations) e2e.remember(conversation);
			}
			return e2e.openConversations(conversations, as);
		},
		getConversation: async (conversationId, as) => {
			const conversation = await inbox.getConversation(conversationId, as);
			if (!as) e2e.remember(conversation, conversationId);
			const [opened] = await e2e.openConversations([conversation], as);
			return opened ?? conversation;
		},
		createConversation: async (request) => {
			const conversation = await inbox.createConversation(request);
			e2e.remember(conversation);
			return conversation;
		},
		listEntries: async (conversationId, query) => {
			const page: IInboxEntriesPage = await inbox.listEntries(
				conversationId,
				query,
			);
			return { ...page, entries: await e2e.open(page.entries, query?.as) };
		},
		sendEntry: (conversationId, request) => e2e.send(conversationId, request),
		listPins: async (conversationId, as) =>
			e2e.openMarks(await inbox.listPins(conversationId, as), as),
		pinMessage: async (conversationId, messageId) => {
			const pin = await inbox.pinMessage(conversationId, messageId);
			return (await e2e.openMarks([pin]))[0] ?? pin;
		},
		starMessage: async (conversationId, messageId) => {
			const star = await inbox.starMessage(conversationId, messageId);
			return (await e2e.openMarks([star]))[0] ?? star;
		},
		listStars: async (query) => {
			const page: IStarredMessagesPage = await inbox.listStars(query);
			return { ...page, stars: await e2e.openMarks(page.stars) };
		},
	};
	return new Proxy(inbox, {
		get(target, property, receiver) {
			if (Object.hasOwn(overrides, property)) {
				return overrides[property as keyof IInboxState];
			}
			const value = Reflect.get(target, property, receiver);
			return typeof value === "function" ? value.bind(target) : value;
		},
	});
}
