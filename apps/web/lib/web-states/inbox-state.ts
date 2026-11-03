import type {
	IConversation,
	IConversationSegmentList,
	ICreateConversationRequest,
	IInboxEntriesPage,
	IInboxEntriesQuery,
	IInboxEntry,
	IInboxImportEntry,
	IInboxImportResult,
	IBotRoomLimits,
	IInboxKey,
	IInboxSegmentsQuery,
	IInboxState,
	IInvitable,
	IInvitation,
	IInviteRequest,
	IInviteResponse,
	IRegisterInboxKeyRequest,
	ISendInboxEntryRequest,
} from "@flow-like/flow-like-ui";
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

	async cancelInvitation(
		conversationId: string,
		principalId: string,
	): Promise<void> {
		await apiDelete(
			`${conversationPath(conversationId)}/invitations/${encodeURIComponent(principalId)}`,
			this.backend.auth,
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
}
