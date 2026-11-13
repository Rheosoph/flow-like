import {
	CONVERSATION_FILE_APP_PREFIX,
	type IConversation,
	type IConversationFileDownload,
	type IInboxEntriesPage,
	type IInboxEntry,
	type IInboxEnvelope,
	type IInboxFileHandle,
	type IInboxKey,
	type IInboxMessageBody,
	type IInboxPlainEnvelope,
	type IInboxSealedEnvelope,
	type IInboxSealedFile,
	type IInboxState,
	type IPinnedMessage,
	type ISendInboxEntryRequest,
	type IStarredMessage,
	type IStarredMessagesPage,
	MAX_CONVERSATION_FILE_BYTES,
} from "../state/backend-state/inbox-state";

/** The key directory's `algorithm` and the sealed envelope's `enc`. */
export const INBOX_KEY_ALGORITHM = "hpke-x25519-sha256-chacha20poly1305";
/** A sealed attachment's `alg`: `flow_like_device_crypto::inbox_file`. */
export const SEALED_FILE_ALGORITHM = "xchacha20poly1305-stream-64k-v1";
/** The name and type a sealed file is uploaded under, so the server learns neither. */
export const SEALED_FILE_NAME = "sealed";
const SEALED_FILE_MIME = "application/octet-stream";
const SEALED_CHUNK_BYTES = 64 * 1024;
const SEALED_NONCE_BYTES = 19;
const SEALED_TAG_BYTES = 16;
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

/** What a sealed bot secret is bound to: it opens only for this bot under this name. */
export interface IBotSecretSealScope {
	bot_id: string;
	name: string;
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
	/** A bot secret's value sealed to the bot's published keys. */
	sealSecret(
		scope: IBotSecretSealScope,
		keys: IInboxSealMember["keys"],
		value: string,
	): Promise<IInboxSealedEnvelope>;
	/** The bodies of `entries`; `null` where none of this device's keys opens one. */
	open(principalId: string, entries: IInboxSealedItem[]): Promise<unknown[]>;
	/** `file` sealed for the conversation under a key made for it alone, base64url. */
	sealFile?(
		conversationId: string,
		file: Uint8Array,
	): Promise<{ key: string; ciphertext: Uint8Array<ArrayBuffer> }>;
	/** A sealed conversation file opened with the key from its message. */
	openFile?(
		conversationId: string,
		key: string,
		ciphertext: Uint8Array,
	): Promise<Uint8Array<ArrayBuffer>>;
}

/** The size of a `bytes`-long file once sealed, as `inbox_file::sealed_len` counts it. */
export function sealedFileBytes(bytes: number): number {
	const chunks = Math.max(1, Math.ceil(bytes / SEALED_CHUNK_BYTES));
	return SEALED_NONCE_BYTES + bytes + chunks * SEALED_TAG_BYTES;
}

/**
 * `file` sealed for `conversationId`: `upload` is the ciphertext under a name and type that say
 * nothing, `sealed` belongs into the attachment part and so only inside the sealed message.
 */
export async function sealConversationFile(
	crypto: IInboxCrypto,
	conversationId: string,
	file: File,
): Promise<{ upload: File; sealed: IInboxSealedFile }> {
	if (!crypto.sealFile) {
		throw new Error(
			`Could not encrypt ${file.name}: this app can't seal files for an encrypted conversation`,
		);
	}
	if (sealedFileBytes(file.size) > MAX_CONVERSATION_FILE_BYTES) {
		throw new Error(
			`Could not encrypt ${file.name}: it has ${file.size} bytes, and sealed it would pass the ${MAX_CONVERSATION_FILE_BYTES}-byte limit of a conversation file`,
		);
	}
	const { key, ciphertext } = await crypto.sealFile(
		conversationId,
		new Uint8Array(await file.arrayBuffer()),
	);
	return {
		upload: new File([ciphertext], SEALED_FILE_NAME, {
			type: SEALED_FILE_MIME,
		}),
		sealed: {
			alg: SEALED_FILE_ALGORITHM,
			key,
			mime: file.type || SEALED_FILE_MIME,
			bytes: file.size,
		},
	};
}

interface ISealedFileRecord extends IInboxSealedFile {
	name: string;
}

/** The sealed attachments a message body carries in `conversationId`, by handle path. */
function sealedFilesOf(
	conversationId: string,
	body: unknown,
): [string, ISealedFileRecord][] {
	const parts = (body as Partial<IInboxMessageBody> | null)?.parts;
	if (!Array.isArray(parts)) return [];
	return parts.flatMap((part) =>
		part?.kind === "attachment" &&
		part.sealed &&
		part.handle?.app_id === `${CONVERSATION_FILE_APP_PREFIX}${conversationId}`
			? [[part.handle.path, { ...part.sealed, name: part.name }]]
			: [],
	);
}

function carriesSealedFile(body: unknown): boolean {
	const parts = (body as Partial<IInboxMessageBody> | null)?.parts;
	return Array.isArray(parts) && parts.some((part) => Boolean(part?.sealed));
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

/** The published keys a secret of `botId` is sealed to, newest last; empty when it has none. */
export function botSecretKeys(
	botId: string,
	keys: readonly IInboxKey[],
): IInboxSealMember["keys"] {
	return sealMembers([botId], keys).get(botId)?.keys ?? [];
}

type ConversationShape = Pick<IConversation, "id" | "encryption" | "members">;

/** Sets `key` in `map`, dropping the oldest entry once the map holds `OPENED_LIMIT`. */
function boundedSet<K, V>(map: Map<K, V>, key: K, value: V): void {
	if (map.size >= OPENED_LIMIT) {
		const oldest = map.keys().next().value;
		if (oldest !== undefined) map.delete(oldest);
	}
	map.set(key, value);
}

/**
 * Seals what the person sends to an end-to-end encrypted conversation and opens what this
 * device's keys can open, so every reader of the inbox sees plain envelopes. Entries sealed to
 * keys this device doesn't hold stay sealed.
 */
export class InboxEndToEnd {
	private readonly opened = new Map<string, unknown>();
	private readonly sealedFiles = new Map<string, ISealedFileRecord>();
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

	private cache(
		entry: Pick<IInboxEntry, "id" | "conversation_id">,
		body: unknown,
	): void {
		boundedSet(this.opened, entry.id, body);
		for (const [path, file] of sealedFilesOf(entry.conversation_id, body)) {
			boundedSet(this.sealedFiles, `${entry.conversation_id}/${path}`, file);
		}
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
		if (conversation.encryption !== "e2e") {
			if (carriesSealedFile(request.envelope.body)) {
				throw new Error(
					"Could not send: an end-to-end encrypted file can't go into a conversation that isn't encrypted; attach the file again",
				);
			}
			return request;
		}
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
			this.cache(entry, request.envelope.body);
			return { ...entry, envelope: plainEnvelope(request.envelope.body) };
		}
		return entry;
	}

	/**
	 * A conversation file to show: a sealed one this device opened a message for is downloaded,
	 * opened here and handed over as a `blob:` link with its own name and type; any other file is
	 * the server's link.
	 */
	async download(
		conversationId: string,
		handle: Pick<IInboxFileHandle, "path" | "mime">,
		as?: string,
	): Promise<IConversationFileDownload> {
		const sealed = this.sealedFiles.get(`${conversationId}/${handle.path}`);
		const reader = this.reader();
		const link = await this.inbox.conversationFileDownload(
			conversationId,
			handle,
			as,
		);
		if (!sealed || (as && as !== reader)) return link;
		if (!this.crypto.openFile) {
			throw new Error(
				`Could not open ${sealed.name}: this app can't open end-to-end encrypted files`,
			);
		}
		const response = await fetch(link.url);
		if (!response.ok) {
			throw new Error(
				`Could not download the encrypted file ${sealed.name}: the server answered ${response.status}`,
			);
		}
		const file = await this.crypto
			.openFile(
				conversationId,
				sealed.key,
				new Uint8Array(await response.arrayBuffer()),
			)
			.catch(() => {
				throw new Error(
					`Could not open the encrypted file ${sealed.name}: it was changed after it was sent, or it belongs to another conversation`,
				);
			});
		return {
			url: URL.createObjectURL(new Blob([file], { type: sealed.mime })),
			name: sealed.name,
			mime: sealed.mime,
			bytes: file.byteLength,
		};
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
					this.cache(entry, bodies[index] ?? null);
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
		conversationFileDownload: (conversationId, handle, as) =>
			e2e.download(conversationId, handle, as),
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
		sealBotSecret: async (botId, name, value) => {
			const keys = botSecretKeys(botId, await inbox.getKeys([botId]));
			if (keys.length === 0) {
				throw new Error(
					`Could not seal secret ${name}: bot ${botId} has published no inbox key. Its host publishes one while it runs with end-to-end encryption.`,
				);
			}
			return crypto.sealSecret({ bot_id: botId, name }, keys, value);
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
