import {
	type IAppVisibility,
	type IMediaItem,
	type ITeamState,
	isAzureBlobStorageUrl,
	toWireVisibility,
} from "@flow-like/flow-like-ui";
import type {
	IAccessibleApp,
	IAppConnectionsResponse,
	IChangeGroupVisibilityResult,
	ICreateGroupPayload,
	IGroup,
	IGroupMembershipRequest,
	IGroupPublicationStatus,
	IInvite,
	IInviteLink,
	IJoinRequest,
	IMember,
	IProcessCaseDetailResponse,
	IProcessCasesResponse,
	IProcessGraphResponse,
	IProcessNote,
	IRemoteEvent,
	IRemoteEventDetail,
	IUpdateGroupPayload,
} from "@flow-like/flow-like-ui/state/backend-state/types";
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
	apiPatch,
	apiPost,
	apiPut,
} from "./api-utils";

export class WebTeamState implements ITeamState {
	constructor(private readonly backend: WebBackendRef) {}

	async createInviteLink(
		appId: string,
		name: string,
		maxUses: number,
		expiresInHours?: number,
	): Promise<void> {
		await apiPut(
			`apps/${appId}/team/link`,
			{ name, max_uses: maxUses, expires_in_hours: expiresInHours ?? null },
			this.backend.auth,
		);
	}

	async getInviteLinks(appId: string): Promise<IInviteLink[]> {
		try {
			return await apiGet<IInviteLink[]>(
				`apps/${appId}/team/link`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async removeInviteLink(appId: string, linkId: string): Promise<void> {
		await apiDelete(`apps/${appId}/team/link/${linkId}`, this.backend.auth);
	}

	async joinInviteLink(appId: string, token: string): Promise<void> {
		await apiPost(
			`apps/${appId}/team/link/join/${token}`,
			undefined,
			this.backend.auth,
		);
	}

	async requestJoin(appId: string, comment: string): Promise<void> {
		await apiPut(`apps/${appId}/team/queue`, { comment }, this.backend.auth);
	}

	async getJoinRequests(
		appId: string,
		offset?: number,
		limit?: number,
	): Promise<IJoinRequest[]> {
		const params = new URLSearchParams();
		if (offset !== undefined) params.set("offset", offset.toString());
		if (limit !== undefined) params.set("limit", limit.toString());

		try {
			return await apiGet<IJoinRequest[]>(
				`apps/${appId}/team/queue?${params}`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async acceptJoinRequest(appId: string, requestId: string): Promise<void> {
		await apiPost(
			`apps/${appId}/team/queue/${requestId}`,
			undefined,
			this.backend.auth,
		);
	}

	async rejectJoinRequest(appId: string, requestId: string): Promise<void> {
		await apiDelete(`apps/${appId}/team/queue/${requestId}`, this.backend.auth);
	}

	async getTeam(
		appId: string,
		offset?: number,
		limit?: number,
	): Promise<IMember[]> {
		const params = new URLSearchParams();
		if (offset !== undefined) params.set("offset", offset.toString());
		if (limit !== undefined) params.set("limit", limit.toString());

		try {
			return await apiGet<IMember[]>(
				`apps/${appId}/team?${params}`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async getInvites(offset?: number, limit?: number): Promise<IInvite[]> {
		const params = new URLSearchParams();
		if (offset !== undefined) params.set("offset", offset.toString());
		if (limit !== undefined) params.set("limit", limit.toString());

		try {
			return await apiGet<IInvite[]>(
				`user/invites?${params}`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async getAppInvites(
		appId: string,
		offset?: number,
		limit?: number,
	): Promise<IInvite[]> {
		const params = new URLSearchParams();
		if (offset !== undefined) params.set("offset", offset.toString());
		if (limit !== undefined) params.set("limit", limit.toString());

		try {
			return await apiGet<IInvite[]>(
				`apps/${appId}/team/invites?${params}`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async revokeAppInvite(appId: string, inviteId: string): Promise<void> {
		await apiDelete(
			`apps/${appId}/team/invites/${inviteId}`,
			this.backend.auth,
		);
	}

	// Accept/decline are POST and DELETE on the invite itself — there are no
	// /accept and /reject sub-paths on the server.
	async acceptInvite(inviteId: string): Promise<void> {
		await apiPost(`user/invites/${inviteId}`, undefined, this.backend.auth);
	}

	async rejectInvite(inviteId: string): Promise<void> {
		await apiDelete(`user/invites/${inviteId}`, this.backend.auth);
	}

	async inviteUser(
		appId: string,
		user_id: string,
		message: string,
	): Promise<void> {
		await apiPut(
			`apps/${appId}/team/invite`,
			{ sub: user_id, message },
			this.backend.auth,
		);
	}

	async removeUser(appId: string, user_id: string): Promise<void> {
		await apiDelete(`apps/${appId}/team/${user_id}`, this.backend.auth);
	}

	async getAppConnections(appId: string): Promise<IAppConnectionsResponse> {
		return await apiGet<IAppConnectionsResponse>(
			`apps/${appId}/connections`,
			this.backend.auth,
		);
	}

	async addAppConnection(
		appId: string,
		sourceAppId: string,
		roleId: string,
	): Promise<void> {
		await apiPost(
			`apps/${appId}/connections`,
			{ source_app_id: sourceAppId, role_id: roleId },
			this.backend.auth,
		);
	}

	async requestAppConnection(
		appId: string,
		targetAppId: string,
		comment?: string,
	): Promise<void> {
		await apiPut(
			`apps/${appId}/connections/request`,
			{ target_app_id: targetAppId, comment },
			this.backend.auth,
		);
	}

	async acceptAppConnection(
		appId: string,
		connectionId: string,
		roleId: string,
	): Promise<void> {
		await apiPost(
			`apps/${appId}/connections/queue/${connectionId}`,
			{ role_id: roleId },
			this.backend.auth,
		);
	}

	async rejectAppConnection(
		appId: string,
		connectionId: string,
	): Promise<void> {
		await apiDelete(
			`apps/${appId}/connections/queue/${connectionId}`,
			this.backend.auth,
		);
	}

	async updateAppConnectionRole(
		appId: string,
		connectionId: string,
		roleId: string,
	): Promise<void> {
		await apiPut(
			`apps/${appId}/connections/${connectionId}`,
			{ role_id: roleId },
			this.backend.auth,
		);
	}

	async removeAppConnection(
		appId: string,
		connectionId: string,
	): Promise<void> {
		await apiDelete(
			`apps/${appId}/connections/${connectionId}`,
			this.backend.auth,
		);
	}

	async getAccessibleApps(appId: string): Promise<IAccessibleApp[]> {
		return await apiGet<IAccessibleApp[]>(
			`apps/${appId}/connections/accessible`,
			this.backend.auth,
		);
	}

	async getRemoteTables(appId: string, targetAppId: string): Promise<string[]> {
		return await apiGet<string[]>(
			`apps/${appId}/connections/${targetAppId}/tables`,
			this.backend.auth,
		);
	}

	async getRemoteEvents(
		appId: string,
		targetAppId: string,
	): Promise<IRemoteEvent[]> {
		return await apiGet<IRemoteEvent[]>(
			`apps/${appId}/connections/${targetAppId}/events`,
			this.backend.auth,
		);
	}

	async getRemoteEventDetail(
		appId: string,
		targetAppId: string,
		eventId: string,
	): Promise<IRemoteEventDetail> {
		return await apiGet<IRemoteEventDetail>(
			`apps/${appId}/connections/${targetAppId}/events/${eventId}/detail`,
			this.backend.auth,
		);
	}

	async getConnectionGraph(
		appId: string,
		days?: number,
	): Promise<IProcessGraphResponse> {
		const params = new URLSearchParams();
		if (days !== undefined) params.set("days", days.toString());
		return await apiGet<IProcessGraphResponse>(
			`apps/${appId}/connections/graph?${params}`,
			this.backend.auth,
		);
	}

	async getProcessCases(
		appId: string,
		days?: number,
	): Promise<IProcessCasesResponse> {
		const params = new URLSearchParams();
		if (days !== undefined) params.set("days", days.toString());
		return await apiGet<IProcessCasesResponse>(
			`apps/${appId}/connections/cases?${params}`,
			this.backend.auth,
		);
	}

	async getProcessCaseRuns(
		appId: string,
		caseId: string,
	): Promise<IProcessCaseDetailResponse> {
		return await apiGet<IProcessCaseDetailResponse>(
			`apps/${appId}/connections/cases/${caseId}`,
			this.backend.auth,
		);
	}

	async getProcessNotes(appId: string): Promise<IProcessNote[]> {
		return await apiGet<IProcessNote[]>(
			`apps/${appId}/connections/notes`,
			this.backend.auth,
		);
	}

	async createProcessNote(
		appId: string,
		content: string,
	): Promise<IProcessNote> {
		return await apiPut<IProcessNote>(
			`apps/${appId}/connections/notes`,
			{ content },
			this.backend.auth,
		);
	}

	async updateProcessNote(
		appId: string,
		noteId: string,
		content: string,
	): Promise<IProcessNote> {
		return await apiPut<IProcessNote>(
			`apps/${appId}/connections/notes/${noteId}`,
			{ content },
			this.backend.auth,
		);
	}

	async deleteProcessNote(appId: string, noteId: string): Promise<void> {
		await apiDelete(
			`apps/${appId}/connections/notes/${noteId}`,
			this.backend.auth,
		);
	}

	async createGroup(
		appId: string,
		payload: ICreateGroupPayload,
	): Promise<IGroup> {
		return await apiPost<IGroup>(
			`apps/${appId}/groups`,
			payload,
			this.backend.auth,
		);
	}

	async listGroups(appId: string): Promise<IGroup[]> {
		return await apiGet<IGroup[]>(`apps/${appId}/groups`, this.backend.auth);
	}

	async getGroup(appId: string, groupId: string): Promise<IGroup> {
		return await apiGet<IGroup>(
			`apps/${appId}/groups/${groupId}`,
			this.backend.auth,
		);
	}

	async updateGroup(
		appId: string,
		groupId: string,
		payload: IUpdateGroupPayload,
	): Promise<IGroup> {
		return await apiPut<IGroup>(
			`apps/${appId}/groups/${groupId}`,
			payload,
			this.backend.auth,
		);
	}

	async deleteGroup(appId: string, groupId: string): Promise<void> {
		await apiDelete(`apps/${appId}/groups/${groupId}`, this.backend.auth);
	}

	async addGroupMember(
		appId: string,
		groupId: string,
		memberAppId: string,
	): Promise<IGroup> {
		return await apiPost<IGroup>(
			`apps/${appId}/groups/${groupId}/members`,
			{ member_app_id: memberAppId },
			this.backend.auth,
		);
	}

	async removeGroupMember(
		appId: string,
		groupId: string,
		memberAppId: string,
	): Promise<void> {
		await apiDelete(
			`apps/${appId}/groups/${groupId}/members/${memberAppId}`,
			this.backend.auth,
		);
	}

	async listGroupRequests(appId: string): Promise<IGroupMembershipRequest[]> {
		return await apiGet<IGroupMembershipRequest[]>(
			`apps/${appId}/groups/requests`,
			this.backend.auth,
		);
	}

	async acceptGroupRequest(appId: string, memberId: string): Promise<void> {
		await apiPost(
			`apps/${appId}/groups/requests/${memberId}`,
			{},
			this.backend.auth,
		);
	}

	async declineGroupRequest(appId: string, memberId: string): Promise<void> {
		await apiDelete(
			`apps/${appId}/groups/requests/${memberId}`,
			this.backend.auth,
		);
	}

	async pushGroupMedia(
		appId: string,
		groupId: string,
		item: IMediaItem,
		file: File,
		language?: string,
	): Promise<void> {
		const params = new URLSearchParams();
		params.set("group_id", groupId);
		params.set("item", item);
		params.set(
			"extension",
			file.name.includes(".") ? (file.name.split(".").pop() ?? "") : "",
		);
		params.set("language", language ?? "en");

		const { signed_url } = await apiPut<{ signed_url: string }>(
			`apps/${appId}/meta/media?${params}`,
			undefined,
			this.backend.auth,
		);

		const headers: HeadersInit = { "Content-Type": file.type };
		// Azure Blob Storage rejects a PUT without this header.
		if (isAzureBlobStorageUrl(signed_url)) {
			headers["x-ms-blob-type"] = "BlockBlob";
		}

		const response = await fetch(signed_url, {
			method: "PUT",
			body: file,
			headers,
		});
		if (!response.ok) {
			throw new Error(`Failed to upload media: ${response.statusText}`);
		}
	}

	async changeGroupVisibility(
		appId: string,
		groupId: string,
		visibility: IAppVisibility,
		message?: string,
	): Promise<IChangeGroupVisibilityResult> {
		return await apiPatch<IChangeGroupVisibilityResult>(
			`apps/${appId}/groups/${groupId}/visibility`,
			{ visibility: toWireVisibility(visibility), message },
			this.backend.auth,
		);
	}

	async getGroupPublication(
		appId: string,
		groupId: string,
	): Promise<IGroupPublicationStatus> {
		return await apiGet<IGroupPublicationStatus>(
			`apps/${appId}/groups/${groupId}/publication`,
			this.backend.auth,
		);
	}

	async leaveGroup(appId: string, groupId: string): Promise<void> {
		await apiDelete(
			`apps/${appId}/groups/${groupId}/membership`,
			this.backend.auth,
		);
	}
}
