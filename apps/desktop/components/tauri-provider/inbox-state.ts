import type {
	IBlockedPrincipal,
	IBotRoomLimits,
	IConversation,
	IConversationFileDownload,
	IConversationFileReady,
	IConversationLink,
	IConversationPinState,
	IConversationSegmentList,
	IConversationTurn,
	ICreateConversationLinkRequest,
	ICreateConversationRequest,
	IInboxEntriesPage,
	IInboxEntriesQuery,
	IInboxEntry,
	IInboxImportEntry,
	IInboxImportResult,
	IInboxKey,
	IInboxSegmentsQuery,
	IInboxState,
	IInvitable,
	IInvitation,
	IInviteRequest,
	IInviteResponse,
	IJoinedConversation,
	IPinnedMessage,
	IRealtimeAccess,
	IRegisterInboxKeyRequest,
	ISendInboxEntryRequest,
	IStarredMessage,
	IStarredMessagesPage,
	IStarredMessagesQuery,
} from "@flow-like/flow-like-ui";
import {
	conversationFileId,
	conversationFileTooLarge,
	putSignedFile,
} from "@flow-like/flow-like-ui/lib/conversation-files";
import { asArray, isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import { LOCAL_OWNER_ID } from "@flow-like/flow-like-ui/state/backend-state/bot-state";
import { inboxPositionParams } from "@flow-like/flow-like-ui/state/backend-state/inbox-state";
import { del, get, patch, post, put } from "../../lib/api";
import {
	AccountListReads,
	type IAccountRoute,
	type IAccountSession,
	accountSession,
} from "../../lib/local-bots/account";
import {
	LOCAL_NOT_FOUND,
	LocalStoreError,
	localInbox,
} from "../../lib/local-bots/commands";
import { mergeConversations } from "../../lib/local-bots/merge";
import { idsOf, localRegistries } from "../../lib/local-bots/registry";
import type { TauriBackend } from "../tauri-provider";

function conversationPath(conversationId: string): string {
	return `inbox/conversations/${encodeURIComponent(conversationId)}`;
}

function withQuery(path: string, params: Record<string, string | undefined>) {
	const search = new URLSearchParams();
	for (const [key, value] of Object.entries(params)) {
		if (value !== undefined && value !== "") search.set(key, value);
	}
	const query = search.toString();
	return query ? `${path}?${query}` : path;
}

function expectRecord<T>(response: unknown, operation: string): T {
	if (!isRecord(response)) {
		throw new Error(`Unexpected response while trying to ${operation}`);
	}
	return response as unknown as T;
}

/** A failing local store must not hide the account's conversations. */
function localConversationsBesideAccount(
	as?: string,
): Promise<IConversation[] | undefined> {
	return localInbox.conversations(as).catch((error) => {
		console.warn("[LocalBots] Listing account conversations only:", error);
		return undefined;
	});
}

const DEFAULT_STARS_PAGE = 50;

/** A failing local store must not hide the account's stars. */
function localStarsBesideAccount(limit: number): Promise<IStarredMessage[]> {
	return localInbox
		.stars({ limit })
		.then((page) => asArray(page.stars))
		.catch((error) => {
			console.warn("[LocalBots] Listing account stars only:", error);
			return [];
		});
}

/** Reading as a bot shows the local conversations that bot is in. */
function readableAs(
	conversations: IConversation[],
	as?: string,
): IConversation[] {
	if (!as) return conversations;
	return conversations.filter((conversation) =>
		asArray(conversation.members).some((member) => member.principal_id === as),
	);
}

/**
 * Signed out, every conversation lives on this computer. Signed in, lists show
 * the account's and this computer's; a conversation between you and bots on
 * this computer stays here until it moves, and its local id then stands for
 * its account id. Local conversations have one reader, so `as` does not change
 * what they return.
 */
export class InboxState implements IInboxState {
	private readonly accountConversations = new AccountListReads<IConversation>();

	constructor(private readonly backend: TauriBackend) {}

	private get registries() {
		return localRegistries(this.backend);
	}

	/** Where calls about this conversation go; undefined when it lives on this computer. */
	private async accountFor(
		conversationId: string,
	): Promise<IAccountRoute | undefined> {
		const session = accountSession(this.backend);
		const ids = this.registries.conversations;
		if (!session || (await ids.isLocal(conversationId, session.sub))) return;
		return { ...session, id: ids.accountId(conversationId, session.sub) };
	}

	private here<T>(conversationId: string, call: () => Promise<T>): Promise<T> {
		return this.registries.conversations.onLocal(conversationId, call);
	}

	private async localPrincipals(
		principalIds: readonly string[],
		sub: string,
	): Promise<string[]> {
		const local = await Promise.all(
			principalIds.map(
				(id) => id === LOCAL_OWNER_ID || this.registries.bots.isLocal(id, sub),
			),
		);
		return principalIds.filter((_, index) => local[index]);
	}

	/** Bots on this computer can't be members of an account conversation. */
	private async refuseLocalMembers(
		members: readonly string[],
		sub: string,
		operation: string,
	): Promise<void> {
		const local = await this.localPrincipals(members, sub);
		if (local.length > 0) {
			throw new Error(
				`Could not ${operation}: ${local.join(", ")} live only on this computer; move them to your account first`,
			);
		}
	}

	private async keySession(
		principalId: string | undefined,
		operation: string,
	): Promise<IAccountSession> {
		const session = accountSession(this.backend);
		if (!session) {
			throw new Error(
				`Could not ${operation}: inbox keys need an account; sign in first`,
			);
		}
		if (
			principalId &&
			(await this.localPrincipals([principalId], session.sub)).length > 0
		) {
			throw new Error(
				`Could not ${operation}: ${principalId} lives only on this computer, and inbox keys need an account`,
			);
		}
		return session;
	}

	async listConversations(as?: string): Promise<IConversation[]> {
		const session = accountSession(this.backend);
		if (!session) {
			const local = await localInbox.conversations(as);
			this.registries.conversations.setLocal(idsOf(local));
			return readableAs(local, as);
		}
		const readsAccount =
			!as || !(await this.registries.bots.isLocal(as, session.sub));
		const [remote, local] = await Promise.all([
			readsAccount
				? this.accountConversations.read(`${session.sub}:${as ?? ""}`, () =>
						this.listAccountConversations(session, as),
					)
				: [],
			as && readsAccount ? [] : localConversationsBesideAccount(as),
		]);
		if (local) this.registries.conversations.setLocal(idsOf(local));
		this.registries.conversations.addRemote(session.sub, idsOf(remote));
		return mergeConversations(remote, readableAs(local ?? [], as));
	}

	private async listAccountConversations(
		session: IAccountSession,
		as?: string,
	): Promise<IConversation[]> {
		const response = await get<{ conversations?: IConversation[] }>(
			session.profile,
			withQuery("inbox/conversations", { as }),
			session.auth,
		);
		return asArray(response?.conversations);
	}

	async getConversation(
		conversationId: string,
		as?: string,
	): Promise<IConversation> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			const conversation = await this.here(conversationId, () =>
				localInbox.conversation(conversationId, as),
			);
			if (conversation) return conversation;
			this.registries.conversations.forgetLocal(conversationId);
			throw new LocalStoreError(
				`Could not load conversation ${conversationId}: it isn't on this computer`,
				LOCAL_NOT_FOUND,
			);
		}
		return expectRecord(
			await get<IConversation>(
				account.profile,
				withQuery(conversationPath(account.id), { as }),
				account.auth,
			),
			`load conversation ${conversationId}`,
		);
	}

	async createConversation(
		request: ICreateConversationRequest,
	): Promise<IConversation> {
		const session = accountSession(this.backend);
		const local = session
			? await this.localPrincipals(request.members, session.sub)
			: request.members;
		const staysLocal =
			!session ||
			(request.members.length > 0 && local.length === request.members.length);
		if (staysLocal) {
			const conversation = await localInbox.create(request);
			this.registries.conversations.addLocal(conversation.id);
			return conversation;
		}
		if (local.length > 0) {
			throw new Error(
				`Could not create a ${request.kind} conversation: ${local.join(", ")} live only on this computer and can't join account members; move them to your account first`,
			);
		}
		const conversation = expectRecord<IConversation>(
			await post<IConversation>(
				session.profile,
				"inbox/conversations",
				request,
				session.auth,
			),
			`create a ${request.kind} conversation`,
		);
		this.registries.conversations.addRemote(session.sub, [conversation.id]);
		return conversation;
	}

	async renameConversation(
		conversationId: string,
		title: string,
	): Promise<IConversation> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			return this.here(conversationId, () =>
				localInbox.rename(conversationId, title),
			);
		}
		return expectRecord(
			await patch<IConversation>(
				account.profile,
				conversationPath(account.id),
				{ title },
				account.auth,
			),
			`rename conversation ${conversationId}`,
		);
	}

	async setConversationImage(
		conversationId: string,
		image: string | null,
	): Promise<IConversation> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			return this.here(conversationId, () =>
				localInbox.setImage(conversationId, image),
			);
		}
		const path = `${conversationPath(account.id)}/image`;
		return expectRecord(
			image === null
				? await del<IConversation>(
						account.profile,
						path,
						undefined,
						account.auth,
					)
				: await put<IConversation>(
						account.profile,
						path,
						{ image },
						account.auth,
					),
			`change the picture of conversation ${conversationId}`,
		);
	}

	async addMembers(
		conversationId: string,
		members: string[],
	): Promise<IConversation> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			return this.here(conversationId, () =>
				localInbox.addMembers(conversationId, members),
			);
		}
		await this.refuseLocalMembers(
			members,
			account.sub,
			`add members to conversation ${conversationId}`,
		);
		return expectRecord(
			await post<IConversation>(
				account.profile,
				`${conversationPath(account.id)}/members`,
				{ members },
				account.auth,
			),
			`add members to conversation ${conversationId}`,
		);
	}

	async removeMember(
		conversationId: string,
		principalId: string,
	): Promise<void> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			// On this computer you are `local`, whichever account is signed in.
			const self = accountSession(this.backend)?.sub;
			return this.here(conversationId, () =>
				localInbox.removeMember(
					conversationId,
					principalId === self ? LOCAL_OWNER_ID : principalId,
				),
			);
		}
		await del<void>(
			account.profile,
			`${conversationPath(account.id)}/members/${encodeURIComponent(principalId)}`,
			undefined,
			account.auth,
		);
	}

	async deleteConversation(conversationId: string): Promise<void> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			return this.here(conversationId, () =>
				localInbox.deleteConversation(conversationId),
			);
		}
		await del<void>(
			account.profile,
			conversationPath(account.id),
			undefined,
			account.auth,
		);
		this.accountConversations.remove(
			`${account.sub}:`,
			(conversation) => conversation.id === account.id,
		);
	}

	async listEntries(
		conversationId: string,
		query: IInboxEntriesQuery = {},
	): Promise<IInboxEntriesPage> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			return this.here(conversationId, () =>
				localInbox.entries(conversationId, query),
			);
		}
		const response = await get<IInboxEntriesPage>(
			account.profile,
			withQuery(`${conversationPath(account.id)}/entries`, {
				as: query.as,
				before: query.before,
				after: query.after,
				limit: query.limit?.toString(),
				...inboxPositionParams(query),
			}),
			account.auth,
		);
		const page = expectRecord<IInboxEntriesPage>(
			response,
			`load entries of conversation ${conversationId}`,
		);
		return {
			entries: asArray(page.entries),
			next_before: page.next_before ?? null,
			next_from_seq: page.next_from_seq ?? null,
			next_to_seq: page.next_to_seq ?? null,
		};
	}

	async importHistory(
		botId: string,
		conversationId: string,
		entries: IInboxImportEntry[],
	): Promise<IInboxImportResult> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			return this.here(conversationId, () =>
				localInbox.importHistory(conversationId, botId, entries),
			);
		}
		const conversation = await this.getConversation(conversationId);
		const owner = conversation.members.find(
			(member) => member.kind === "user",
		)?.principal_id;
		if (!owner) {
			throw new Error(
				`Conversation ${conversationId} has no owner to import as`,
			);
		}
		const response = await post<{ imported?: number; skipped?: number }>(
			account.profile,
			"inbox/import",
			{
				local_id: `botchat-${botId}`,
				kind: "direct",
				members: [botId],
				entries: entries.map((entry) => ({
					message_id: entry.message_id,
					sender_id: entry.sender === "bot" ? botId : owner,
					kind: "message",
					envelope: entry.envelope,
					created_at: entry.created_at,
				})),
			},
			account.auth,
		);
		return {
			imported: response?.imported ?? 0,
			skipped: response?.skipped ?? 0,
		};
	}

	async listSegments(
		conversationId: string,
		query: IInboxSegmentsQuery = {},
	): Promise<IConversationSegmentList> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			return this.here(conversationId, () =>
				localInbox.segments(conversationId, query),
			);
		}
		const list = expectRecord<IConversationSegmentList>(
			await get<IConversationSegmentList>(
				account.profile,
				withQuery(`${conversationPath(account.id)}/segments`, {
					as: query.as,
					level: query.level?.toString(),
					after: query.after,
					limit: query.limit?.toString(),
				}),
				account.auth,
			),
			`load segments of conversation ${conversationId}`,
		);
		return {
			segments: asArray(list.segments),
			next_after: list.next_after ?? null,
		};
	}

	async sendEntry(
		conversationId: string,
		request: ISendInboxEntryRequest,
	): Promise<IInboxEntry> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			return this.here(conversationId, () =>
				localInbox.send(conversationId, request),
			);
		}
		return expectRecord(
			await post<IInboxEntry>(
				account.profile,
				`${conversationPath(account.id)}/entries`,
				request,
				account.auth,
			),
			`send a ${request.kind} to conversation ${conversationId}`,
		);
	}

	async uploadConversationFile(
		conversationId: string,
		file: File,
	): Promise<IConversationFileReady> {
		const tooLarge = conversationFileTooLarge(file);
		if (tooLarge) throw new Error(tooLarge);
		const account = await this.accountFor(conversationId);
		if (!account) {
			return this.here(conversationId, () =>
				localInbox.uploadFile(conversationId, file),
			);
		}
		const upload = expectRecord<{ file_id: string; upload_url: string }>(
			await post(
				account.profile,
				`${conversationPath(account.id)}/files`,
				{ name: file.name, mime: file.type || undefined, bytes: file.size },
				account.auth,
			),
			`start the upload of ${file.name} to conversation ${conversationId}`,
		);
		await putSignedFile(upload.upload_url, file);
		return expectRecord<IConversationFileReady>(
			await post(
				account.profile,
				`${conversationPath(account.id)}/files/${encodeURIComponent(upload.file_id)}/complete`,
				{},
				account.auth,
			),
			`finish the upload of ${file.name} to conversation ${conversationId}`,
		);
	}

	async conversationFileDownload(
		conversationId: string,
		handle: { path: string; mime: string },
		as?: string,
	): Promise<IConversationFileDownload> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			return this.here(conversationId, () =>
				localInbox.fileUrl(conversationId, handle, undefined, as),
			);
		}
		return expectRecord(
			await get<IConversationFileDownload>(
				account.profile,
				withQuery(
					`${conversationPath(account.id)}/files/${encodeURIComponent(conversationFileId(handle.path))}`,
					{ as },
				),
				account.auth,
			),
			`download a file of conversation ${conversationId}`,
		);
	}

	async setConversationPinned(
		conversationId: string,
		pinned: boolean,
	): Promise<IConversationPinState> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			return this.here(conversationId, () =>
				localInbox.setPinned(conversationId, pinned),
			);
		}
		const state = expectRecord<IConversationPinState>(
			await patch<IConversationPinState>(
				account.profile,
				`${conversationPath(account.id)}/pin`,
				{ pinned },
				account.auth,
			),
			`${pinned ? "pin" : "unpin"} conversation ${conversationId}`,
		);
		return { ...state, conversation_id: conversationId };
	}

	async listPins(
		conversationId: string,
		as?: string,
	): Promise<IPinnedMessage[]> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			return this.here(conversationId, () =>
				localInbox.pins(conversationId, as),
			);
		}
		const response = expectRecord<{ pins?: IPinnedMessage[] }>(
			await get<{ pins?: IPinnedMessage[] }>(
				account.profile,
				withQuery(`${conversationPath(account.id)}/pins`, { as }),
				account.auth,
			),
			`load pinned messages of conversation ${conversationId}`,
		);
		return asArray(response.pins);
	}

	async pinMessage(
		conversationId: string,
		messageId: string,
	): Promise<IPinnedMessage> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			return this.here(conversationId, () =>
				localInbox.pinMessage(conversationId, messageId),
			);
		}
		return expectRecord(
			await put<IPinnedMessage>(
				account.profile,
				`${conversationPath(account.id)}/pins/${encodeURIComponent(messageId)}`,
				{},
				account.auth,
			),
			`pin ${messageId} in conversation ${conversationId}`,
		);
	}

	async unpinMessage(conversationId: string, messageId: string): Promise<void> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			return this.here(conversationId, () =>
				localInbox.unpinMessage(conversationId, messageId),
			);
		}
		await del<void>(
			account.profile,
			`${conversationPath(account.id)}/pins/${encodeURIComponent(messageId)}`,
			undefined,
			account.auth,
		);
	}

	async starMessage(
		conversationId: string,
		messageId: string,
	): Promise<IStarredMessage> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			return this.here(conversationId, () =>
				localInbox.star(conversationId, messageId),
			);
		}
		const star = expectRecord<IStarredMessage>(
			await put<IStarredMessage>(
				account.profile,
				`${conversationPath(account.id)}/stars/${encodeURIComponent(messageId)}`,
				{},
				account.auth,
			),
			`star ${messageId} in conversation ${conversationId}`,
		);
		return { ...star, conversation_id: conversationId };
	}

	async unstarMessage(
		conversationId: string,
		messageId: string,
	): Promise<void> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			return this.here(conversationId, () =>
				localInbox.unstar(conversationId, messageId),
			);
		}
		await del<void>(
			account.profile,
			`${conversationPath(account.id)}/stars/${encodeURIComponent(messageId)}`,
			undefined,
			account.auth,
		);
	}

	async listStars(
		query: IStarredMessagesQuery = {},
	): Promise<IStarredMessagesPage> {
		const session = accountSession(this.backend);
		if (query.conversationId) {
			return this.listConversationStars(query.conversationId, query);
		}
		if (!session) return this.localStars(query);
		const limit = query.limit ?? DEFAULT_STARS_PAGE;
		const [local, remote] = await Promise.all([
			query.before ? [] : localStarsBesideAccount(limit),
			this.accountStars(session, query),
		]);
		const ids = this.registries.conversations;
		const accountStars = [];
		for (const star of remote.stars) {
			if (!(await ids.isLocal(star.conversation_id, session.sub))) {
				accountStars.push(star);
			}
		}
		const stars = [...local, ...accountStars].sort(
			(a, b) => Date.parse(b.starred_at) - Date.parse(a.starred_at),
		);
		return { stars, next_before: remote.next_before };
	}

	private async listConversationStars(
		conversationId: string,
		query: IStarredMessagesQuery,
	): Promise<IStarredMessagesPage> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			return this.here(conversationId, () => this.localStars(query));
		}
		const page = await this.accountStars(account, {
			...query,
			conversationId: account.id,
		});
		return {
			...page,
			stars: page.stars.map((star) => ({
				...star,
				conversation_id: conversationId,
			})),
		};
	}

	private async localStars(
		query: IStarredMessagesQuery,
	): Promise<IStarredMessagesPage> {
		const page = await localInbox.stars(query);
		return {
			stars: asArray(page.stars),
			next_before: page.next_before ?? null,
		};
	}

	private async accountStars(
		session: IAccountSession,
		query: IStarredMessagesQuery,
	): Promise<IStarredMessagesPage> {
		const page = expectRecord<IStarredMessagesPage>(
			await get<IStarredMessagesPage>(
				session.profile,
				withQuery("inbox/stars", {
					conversation_id: query.conversationId,
					before: query.before,
					limit: query.limit?.toString(),
				}),
				session.auth,
			),
			"load starred messages",
		);
		return {
			stars: asArray(page.stars),
			next_before: page.next_before ?? null,
		};
	}

	async markRead(
		conversationId: string,
		upTo?: string,
	): Promise<{ last_read_at: string }> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			return this.here(conversationId, () =>
				localInbox.markRead(conversationId, upTo),
			);
		}
		return expectRecord(
			await post<{ last_read_at: string }>(
				account.profile,
				`${conversationPath(account.id)}/read`,
				upTo ? { up_to: upTo } : {},
				account.auth,
			),
			`mark conversation ${conversationId} as read`,
		);
	}

	/**
	 * Principals on this computer have no inbox keys. One the store can't place is asked of the
	 * account: keys are public, and conversations on this computer are never encrypted.
	 */
	async getKeys(principalIds: string[]): Promise<IInboxKey[]> {
		const session = accountSession(this.backend);
		if (!session || principalIds.length === 0) return [];
		const placed = await Promise.all(
			principalIds.map(
				(id) =>
					id === LOCAL_OWNER_ID ||
					this.registries.bots.isLocal(id, session.sub).catch(() => false),
			),
		);
		const remote = principalIds.filter((_, index) => !placed[index]);
		if (remote.length === 0) return [];
		const response = await get<{ keys?: IInboxKey[] }>(
			session.profile,
			withQuery("inbox/keys", { principals: remote.join(",") }),
			session.auth,
		);
		return asArray(response?.keys);
	}

	async registerKey(request: IRegisterInboxKeyRequest): Promise<IInboxKey> {
		const operation = `register inbox key ${request.key_id}`;
		const session = await this.keySession(request.principal_id, operation);
		return expectRecord(
			await put<IInboxKey>(
				session.profile,
				"inbox/keys",
				request,
				session.auth,
			),
			operation,
		);
	}

	private invitationSession(operation: string): IAccountSession {
		const session = accountSession(this.backend);
		if (!session) {
			throw new Error(
				`Could not ${operation}: invitations need an account; sign in first`,
			);
		}
		return session;
	}

	async listInvitations(): Promise<IInvitation[]> {
		const session = accountSession(this.backend);
		if (!session) return [];
		const response = await get<{ invitations?: IInvitation[] }>(
			session.profile,
			"inbox/invitations",
			session.auth,
		);
		return asArray(response?.invitations);
	}

	async listBlocks(): Promise<IBlockedPrincipal[]> {
		const session = accountSession(this.backend);
		if (!session) return [];
		const response = await get<{ blocks?: IBlockedPrincipal[] }>(
			session.profile,
			"inbox/blocks",
			session.auth,
		);
		return asArray(response?.blocks);
	}

	async blockPrincipal(principalId: string): Promise<void> {
		const session = accountSession(this.backend);
		if (!session) throw new Error("Sign in to block a person or bot.");
		await put(
			session.profile,
			`inbox/blocks/${encodeURIComponent(principalId)}`,
			{},
			session.auth,
		);
	}

	async unblockPrincipal(principalId: string): Promise<void> {
		const session = accountSession(this.backend);
		if (!session) throw new Error("Sign in to unblock a person or bot.");
		await del<void>(
			session.profile,
			`inbox/blocks/${encodeURIComponent(principalId)}`,
			undefined,
			session.auth,
		);
	}

	async listInvitable(
		query: { q?: string; conversationId?: string } = {},
	): Promise<IInvitable[]> {
		const session = accountSession(this.backend);
		if (!session) return [];
		const response = await get<{ principals?: IInvitable[] }>(
			session.profile,
			withQuery("inbox/invitable", {
				q: query.q,
				conversation_id: query.conversationId,
			}),
			session.auth,
		);
		return asArray(response?.principals);
	}

	async invite(
		conversationId: string,
		request: IInviteRequest,
	): Promise<IInviteResponse> {
		const operation = `invite to conversation ${conversationId}`;
		const session = this.invitationSession(operation);
		return expectRecord(
			await post<IInviteResponse>(
				session.profile,
				`${conversationPath(conversationId)}/invitations`,
				request,
				session.auth,
			),
			operation,
		);
	}

	async acceptInvitation(
		invitation: IInvitation,
		limits?: IBotRoomLimits | null,
	): Promise<void> {
		const session = this.invitationSession(
			`accept invitation ${invitation.id}`,
		);
		await post(
			session.profile,
			`inbox/invitations/${encodeURIComponent(invitation.id)}/accept`,
			{ limits: limits ?? null },
			session.auth,
		);
	}

	async declineInvitation(invitation: IInvitation): Promise<void> {
		const session = this.invitationSession(
			`decline invitation ${invitation.id}`,
		);
		await post(
			session.profile,
			`inbox/invitations/${encodeURIComponent(invitation.id)}/decline`,
			{},
			session.auth,
		);
	}

	async blockInvitation(invitation: IInvitation): Promise<void> {
		const session = this.invitationSession(`block invitation ${invitation.id}`);
		await post(
			session.profile,
			`inbox/invitations/${encodeURIComponent(invitation.id)}/block`,
			{},
			session.auth,
		);
	}

	async setConversationMuted(
		conversationId: string,
		muted: boolean,
	): Promise<void> {
		const session = this.invitationSession(`mute ${conversationId}`);
		await patch(
			session.profile,
			`${conversationPath(conversationId)}/mute`,
			{ muted },
			session.auth,
		);
	}

	async cancelInvitation(
		conversationId: string,
		principalId: string,
	): Promise<void> {
		const session = this.invitationSession(
			`cancel the invitation of ${principalId}`,
		);
		await del<void>(
			session.profile,
			`${conversationPath(conversationId)}/invitations/${encodeURIComponent(principalId)}`,
			undefined,
			session.auth,
		);
	}

	async createConversationLink(
		conversationId: string,
		request: ICreateConversationLinkRequest,
	): Promise<IConversationLink> {
		const operation = `create an invite link for conversation ${conversationId}`;
		const session = this.invitationSession(operation);
		return expectRecord(
			await post<IConversationLink>(
				session.profile,
				`${conversationPath(conversationId)}/links`,
				request,
				session.auth,
			),
			operation,
		);
	}

	async listConversationLinks(
		conversationId: string,
	): Promise<IConversationLink[]> {
		const session = accountSession(this.backend);
		if (!session) return [];
		const response = await get<{ links?: IConversationLink[] }>(
			session.profile,
			`${conversationPath(conversationId)}/links`,
			session.auth,
		);
		return asArray(response?.links);
	}

	async revokeConversationLink(
		conversationId: string,
		linkId: string,
	): Promise<void> {
		const session = this.invitationSession(`revoke invite link ${linkId}`);
		await del<void>(
			session.profile,
			`${conversationPath(conversationId)}/links/${encodeURIComponent(linkId)}`,
			undefined,
			session.auth,
		);
	}

	async joinConversationLink(token: string): Promise<IJoinedConversation> {
		const operation = "join a conversation by invite link";
		const session = this.invitationSession(operation);
		return expectRecord(
			await post<IJoinedConversation>(
				session.profile,
				"inbox/links/join",
				{ token },
				session.auth,
			),
			operation,
		);
	}

	async setMemberLimits(
		conversationId: string,
		botId: string,
		limits: IBotRoomLimits | null,
	): Promise<void> {
		const session = this.invitationSession(`set the room limits of ${botId}`);
		await patch(
			session.profile,
			`${conversationPath(conversationId)}/members/${encodeURIComponent(botId)}/limits`,
			{ limits },
			session.auth,
		);
	}

	async revokeKey(keyId: string, principalId?: string): Promise<void> {
		const session = await this.keySession(
			principalId,
			`revoke inbox key ${keyId}`,
		);
		await del<void>(
			session.profile,
			withQuery(`inbox/keys/${encodeURIComponent(keyId)}`, {
				principal_id: principalId,
			}),
			undefined,
			session.auth,
		);
	}

	async listConversationTurns(
		conversationId: string,
	): Promise<IConversationTurn[]> {
		const account = await this.accountFor(conversationId);
		if (!account) return [];
		const response = expectRecord<{ turns?: IConversationTurn[] }>(
			await get<{ turns?: IConversationTurn[] }>(
				account.profile,
				`${conversationPath(account.id)}/turns`,
				account.auth,
			),
			`list turns of conversation ${conversationId}`,
		);
		return asArray(response.turns);
	}

	async getConversationRealtimeAccess(
		conversationId: string,
	): Promise<IRealtimeAccess> {
		const account = await this.accountFor(conversationId);
		if (!account) {
			throw new Error(
				`Realtime access is unavailable for local conversation ${conversationId}`,
			);
		}
		return expectRecord(
			await post<IRealtimeAccess>(
				account.profile,
				`${conversationPath(account.id)}/realtime`,
				undefined,
				account.auth,
			),
			`get realtime access for conversation ${conversationId}`,
		);
	}
}
