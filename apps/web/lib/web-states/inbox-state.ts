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
import { inboxPositionParams } from "@flow-like/flow-like-ui/state/backend-state/inbox-state";
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
	apiPatch,
	apiPost,
	apiPut,
} from "./api-utils";

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

export class WebInboxState implements IInboxState {
	constructor(private readonly backend: WebBackendRef) {}

	async listConversations(as?: string): Promise<IConversation[]> {
		const response = await apiGet<{ conversations?: IConversation[] }>(
			withQuery("inbox/conversations", { as }),
			this.backend.auth,
		);
		return asArray(response?.conversations);
	}

	async getConversation(
		conversationId: string,
		as?: string,
	): Promise<IConversation> {
		return expectRecord(
			await apiGet<IConversation>(
				withQuery(conversationPath(conversationId), { as }),
				this.backend.auth,
			),
			`load conversation ${conversationId}`,
		);
	}

	async createConversation(
		request: ICreateConversationRequest,
	): Promise<IConversation> {
		return expectRecord(
			await apiPost<IConversation>(
				"inbox/conversations",
				request,
				this.backend.auth,
			),
			`create a ${request.kind} conversation`,
		);
	}

	async renameConversation(
		conversationId: string,
		title: string,
	): Promise<IConversation> {
		return expectRecord(
			await apiPatch<IConversation>(
				conversationPath(conversationId),
				{ title },
				this.backend.auth,
			),
			`rename conversation ${conversationId}`,
		);
	}

	async deleteConversation(conversationId: string): Promise<void> {
		await apiDelete(conversationPath(conversationId), this.backend.auth);
	}

	async setConversationImage(
		conversationId: string,
		image: string | null,
	): Promise<IConversation> {
		const path = `${conversationPath(conversationId)}/image`;
		return expectRecord(
			image === null
				? await apiDelete<IConversation>(path, this.backend.auth)
				: await apiPut<IConversation>(path, { image }, this.backend.auth),
			`change the picture of conversation ${conversationId}`,
		);
	}

	async addMembers(
		conversationId: string,
		members: string[],
	): Promise<IConversation> {
		return expectRecord(
			await apiPost<IConversation>(
				`${conversationPath(conversationId)}/members`,
				{ members },
				this.backend.auth,
			),
			`add members to conversation ${conversationId}`,
		);
	}

	async removeMember(
		conversationId: string,
		principalId: string,
	): Promise<void> {
		await apiDelete(
			`${conversationPath(conversationId)}/members/${encodeURIComponent(principalId)}`,
			this.backend.auth,
		);
	}

	async listEntries(
		conversationId: string,
		query: IInboxEntriesQuery = {},
	): Promise<IInboxEntriesPage> {
		const response = await apiGet<IInboxEntriesPage>(
			withQuery(`${conversationPath(conversationId)}/entries`, {
				as: query.as,
				before: query.before,
				after: query.after,
				limit: query.limit?.toString(),
				...inboxPositionParams(query),
			}),
			this.backend.auth,
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
		const conversation = await this.getConversation(conversationId);
		const owner = conversation.members.find(
			(member) => member.kind === "user",
		)?.principal_id;
		if (!owner) {
			throw new Error(
				`Conversation ${conversationId} has no owner to import as`,
			);
		}
		const response = await apiPost<{ imported?: number; skipped?: number }>(
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
			this.backend.auth,
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
		const response = await apiGet<IConversationSegmentList>(
			withQuery(`${conversationPath(conversationId)}/segments`, {
				as: query.as,
				level: query.level?.toString(),
				after: query.after,
				limit: query.limit?.toString(),
			}),
			this.backend.auth,
		);
		const list = expectRecord<IConversationSegmentList>(
			response,
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
		return expectRecord(
			await apiPost<IInboxEntry>(
				`${conversationPath(conversationId)}/entries`,
				request,
				this.backend.auth,
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
		const upload = expectRecord<{ file_id: string; upload_url: string }>(
			await apiPost(
				`${conversationPath(conversationId)}/files`,
				{ name: file.name, mime: file.type || undefined, bytes: file.size },
				this.backend.auth,
			),
			`start the upload of ${file.name} to conversation ${conversationId}`,
		);
		await putSignedFile(upload.upload_url, file);
		return expectRecord<IConversationFileReady>(
			await apiPost(
				`${conversationPath(conversationId)}/files/${encodeURIComponent(upload.file_id)}/complete`,
				{},
				this.backend.auth,
			),
			`finish the upload of ${file.name} to conversation ${conversationId}`,
		);
	}

	async conversationFileDownload(
		conversationId: string,
		handle: { path: string },
		as?: string,
	): Promise<IConversationFileDownload> {
		return expectRecord(
			await apiGet<IConversationFileDownload>(
				withQuery(
					`${conversationPath(conversationId)}/files/${encodeURIComponent(conversationFileId(handle.path))}`,
					{ as },
				),
				this.backend.auth,
			),
			`download a file of conversation ${conversationId}`,
		);
	}

	async markRead(
		conversationId: string,
		upTo?: string,
	): Promise<{ last_read_at: string }> {
		return expectRecord(
			await apiPost<{ last_read_at: string }>(
				`${conversationPath(conversationId)}/read`,
				upTo ? { up_to: upTo } : {},
				this.backend.auth,
			),
			`mark conversation ${conversationId} as read`,
		);
	}

	async getKeys(principalIds: string[]): Promise<IInboxKey[]> {
		if (principalIds.length === 0) return [];
		const response = await apiGet<{ keys?: IInboxKey[] }>(
			withQuery("inbox/keys", { principals: principalIds.join(",") }),
			this.backend.auth,
		);
		return asArray(response?.keys);
	}

	async registerKey(request: IRegisterInboxKeyRequest): Promise<IInboxKey> {
		return expectRecord(
			await apiPut<IInboxKey>("inbox/keys", request, this.backend.auth),
			`register inbox key ${request.key_id}`,
		);
	}

	async listInvitations(): Promise<IInvitation[]> {
		const response = await apiGet<{ invitations?: IInvitation[] }>(
			"inbox/invitations",
			this.backend.auth,
		);
		return asArray(response?.invitations);
	}

	async listBlocks(): Promise<IBlockedPrincipal[]> {
		const response = await apiGet<{ blocks?: IBlockedPrincipal[] }>(
			"inbox/blocks",
			this.backend.auth,
		);
		return asArray(response?.blocks);
	}

	async blockPrincipal(principalId: string): Promise<void> {
		await apiPut(
			`inbox/blocks/${encodeURIComponent(principalId)}`,
			{},
			this.backend.auth,
		);
	}

	async unblockPrincipal(principalId: string): Promise<void> {
		await apiDelete(
			`inbox/blocks/${encodeURIComponent(principalId)}`,
			this.backend.auth,
		);
	}

	async listInvitable(
		query: { q?: string; conversationId?: string } = {},
	): Promise<IInvitable[]> {
		const response = await apiGet<{ principals?: IInvitable[] }>(
			withQuery("inbox/invitable", {
				q: query.q,
				conversation_id: query.conversationId,
			}),
			this.backend.auth,
		);
		return asArray(response?.principals);
	}

	async invite(
		conversationId: string,
		request: IInviteRequest,
	): Promise<IInviteResponse> {
		return expectRecord(
			await apiPost<IInviteResponse>(
				`${conversationPath(conversationId)}/invitations`,
				request,
				this.backend.auth,
			),
			`invite to conversation ${conversationId}`,
		);
	}

	async acceptInvitation(
		invitation: IInvitation,
		limits?: IBotRoomLimits | null,
	): Promise<void> {
		await apiPost(
			`inbox/invitations/${encodeURIComponent(invitation.id)}/accept`,
			{ limits: limits ?? null },
			this.backend.auth,
		);
	}

	async declineInvitation(invitation: IInvitation): Promise<void> {
		await apiPost(
			`inbox/invitations/${encodeURIComponent(invitation.id)}/decline`,
			{},
			this.backend.auth,
		);
	}

	async blockInvitation(invitation: IInvitation): Promise<void> {
		await apiPost(
			`inbox/invitations/${encodeURIComponent(invitation.id)}/block`,
			{},
			this.backend.auth,
		);
	}

	async setConversationMuted(
		conversationId: string,
		muted: boolean,
	): Promise<void> {
		await apiPatch(
			`${conversationPath(conversationId)}/mute`,
			{ muted },
			this.backend.auth,
		);
	}

	async setConversationPinned(
		conversationId: string,
		pinned: boolean,
	): Promise<IConversationPinState> {
		return expectRecord(
			await apiPatch<IConversationPinState>(
				`${conversationPath(conversationId)}/pin`,
				{ pinned },
				this.backend.auth,
			),
			`${pinned ? "pin" : "unpin"} conversation ${conversationId}`,
		);
	}

	async listPins(
		conversationId: string,
		as?: string,
	): Promise<IPinnedMessage[]> {
		const response = await apiGet<{ pins?: IPinnedMessage[] }>(
			withQuery(`${conversationPath(conversationId)}/pins`, { as }),
			this.backend.auth,
		);
		return asArray(response?.pins);
	}

	async pinMessage(
		conversationId: string,
		messageId: string,
	): Promise<IPinnedMessage> {
		return expectRecord(
			await apiPut<IPinnedMessage>(
				`${conversationPath(conversationId)}/pins/${encodeURIComponent(messageId)}`,
				{},
				this.backend.auth,
			),
			`pin message ${messageId} in conversation ${conversationId}`,
		);
	}

	async unpinMessage(conversationId: string, messageId: string): Promise<void> {
		await apiDelete(
			`${conversationPath(conversationId)}/pins/${encodeURIComponent(messageId)}`,
			this.backend.auth,
		);
	}

	async starMessage(
		conversationId: string,
		messageId: string,
	): Promise<IStarredMessage> {
		return expectRecord(
			await apiPut<IStarredMessage>(
				`${conversationPath(conversationId)}/stars/${encodeURIComponent(messageId)}`,
				{},
				this.backend.auth,
			),
			`star message ${messageId} in conversation ${conversationId}`,
		);
	}

	async unstarMessage(
		conversationId: string,
		messageId: string,
	): Promise<void> {
		await apiDelete(
			`${conversationPath(conversationId)}/stars/${encodeURIComponent(messageId)}`,
			this.backend.auth,
		);
	}

	async listStars(
		query: IStarredMessagesQuery = {},
	): Promise<IStarredMessagesPage> {
		const response = await apiGet<IStarredMessagesPage>(
			withQuery("inbox/stars", {
				conversation_id: query.conversationId,
				before: query.before,
				limit: query.limit?.toString(),
			}),
			this.backend.auth,
		);
		const page = expectRecord<IStarredMessagesPage>(
			response,
			"load starred messages",
		);
		return {
			stars: asArray(page.stars),
			next_before: page.next_before ?? null,
		};
	}

	async cancelInvitation(
		conversationId: string,
		principalId: string,
	): Promise<void> {
		await apiDelete(
			`${conversationPath(conversationId)}/invitations/${encodeURIComponent(principalId)}`,
			this.backend.auth,
		);
	}

	async createConversationLink(
		conversationId: string,
		request: ICreateConversationLinkRequest,
	): Promise<IConversationLink> {
		return expectRecord(
			await apiPost<IConversationLink>(
				`${conversationPath(conversationId)}/links`,
				request,
				this.backend.auth,
			),
			`create an invite link for conversation ${conversationId}`,
		);
	}

	async listConversationLinks(
		conversationId: string,
	): Promise<IConversationLink[]> {
		const response = await apiGet<{ links?: IConversationLink[] }>(
			`${conversationPath(conversationId)}/links`,
			this.backend.auth,
		);
		return asArray(response?.links);
	}

	async revokeConversationLink(
		conversationId: string,
		linkId: string,
	): Promise<void> {
		await apiDelete(
			`${conversationPath(conversationId)}/links/${encodeURIComponent(linkId)}`,
			this.backend.auth,
		);
	}

	async joinConversationLink(token: string): Promise<IJoinedConversation> {
		return expectRecord(
			await apiPost<IJoinedConversation>(
				"inbox/links/join",
				{ token },
				this.backend.auth,
			),
			"join a conversation by invite link",
		);
	}

	async setMemberLimits(
		conversationId: string,
		botId: string,
		limits: IBotRoomLimits | null,
	): Promise<void> {
		await apiPatch(
			`${conversationPath(conversationId)}/members/${encodeURIComponent(botId)}/limits`,
			{ limits },
			this.backend.auth,
		);
	}

	async revokeKey(keyId: string, principalId?: string): Promise<void> {
		await apiDelete(
			withQuery(`inbox/keys/${encodeURIComponent(keyId)}`, {
				principal_id: principalId,
			}),
			this.backend.auth,
		);
	}

	async listConversationTurns(
		conversationId: string,
	): Promise<IConversationTurn[]> {
		const response = await apiGet<{ turns?: IConversationTurn[] }>(
			`${conversationPath(conversationId)}/turns`,
			this.backend.auth,
		);
		return asArray(
			expectRecord<{ turns?: IConversationTurn[] }>(response, "list turns")
				.turns,
		);
	}

	async getConversationRealtimeAccess(
		conversationId: string,
	): Promise<IRealtimeAccess> {
		return expectRecord(
			await apiPost<IRealtimeAccess>(
				`${conversationPath(conversationId)}/realtime`,
				undefined,
				this.backend.auth,
			),
			`get realtime access for conversation ${conversationId}`,
		);
	}
}
