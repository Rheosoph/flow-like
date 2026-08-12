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
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

export class TeamState implements ITeamState {
	constructor(private readonly backend: TauriBackend) {}
	async createInviteLink(
		appId: string,
		name: string,
		maxUses: number,
		expiresInHours?: number,
	): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/team/link`,
			{
				method: "PUT",
				body: JSON.stringify({
					name: name,
					max_uses: maxUses,
					expires_in_hours: expiresInHours ?? null,
				}),
			},
			this.backend.auth,
		);
	}
	async getInviteLinks(appId: string): Promise<IInviteLink[]> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		return await fetcher(
			this.backend.profile,
			`apps/${appId}/team/link`,
			{
				method: "GET",
			},
			this.backend.auth,
		);
	}
	async removeInviteLink(appId: string, linkId: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/team/link/${linkId}`,
			{
				method: "DELETE",
			},
			this.backend.auth,
		);
	}
	async joinInviteLink(appId: string, token: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/team/link/join/${token}`,
			{
				method: "POST",
			},
			this.backend.auth,
		);
	}
	async requestJoin(appId: string, comment: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/team/queue`,
			{
				method: "PUT",
				body: JSON.stringify({
					comment: comment,
				}),
			},
			this.backend.auth,
		);
	}
	async getJoinRequests(
		appId: string,
		offset?: number,
		limit?: number,
	): Promise<IJoinRequest[]> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		let url = `apps/${appId}/team/queue`;

		const effectiveOffset = offset ?? 0;
		if (limit) {
			url += `?offset=${effectiveOffset}&limit=${limit}`;
		}

		return await fetcher(
			this.backend.profile,
			url,
			{
				method: "GET",
			},
			this.backend.auth,
		);
	}
	async acceptJoinRequest(appId: string, requestId: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/team/queue/${requestId}`,
			{
				method: "POST",
			},
			this.backend.auth,
		);
	}
	async rejectJoinRequest(appId: string, requestId: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/team/queue/${requestId}`,
			{
				method: "DELETE",
			},
			this.backend.auth,
		);
	}
	async getTeam(
		appId: string,
		offset?: number,
		limit?: number,
	): Promise<IMember[]> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		let url = `apps/${appId}/team`;
		const effectiveOffset = offset ?? 0;
		const effectiveLimit = limit ?? 20;
		if (effectiveLimit) {
			url += `?offset=${effectiveOffset}&limit=${effectiveLimit}`;
		}

		return await fetcher(
			this.backend.profile,
			url,
			{
				method: "GET",
			},
			this.backend.auth,
		);
	}
	async getInvites(offset?: number, limit?: number): Promise<IInvite[]> {
		// Return empty if not authenticated (invites require auth)
		if (!this.backend.profile || !this.backend.auth) {
			return [];
		}

		let url = "user/invites";
		const effectiveOffset = offset ?? 0;
		const effectiveLimit = limit ?? 20;
		if (effectiveLimit) {
			url += `?offset=${effectiveOffset}&limit=${effectiveLimit}`;
		}

		return await fetcher(
			this.backend.profile,
			url,
			{
				method: "GET",
			},
			this.backend.auth,
		);
	}
	async getAppInvites(
		appId: string,
		offset?: number,
		limit?: number,
	): Promise<IInvite[]> {
		if (!this.backend.profile || !this.backend.auth) {
			return [];
		}

		let url = `apps/${appId}/team/invites`;
		const effectiveOffset = offset ?? 0;
		const effectiveLimit = limit ?? 20;
		if (effectiveLimit) {
			url += `?offset=${effectiveOffset}&limit=${effectiveLimit}`;
		}

		return await fetcher(
			this.backend.profile,
			url,
			{
				method: "GET",
			},
			this.backend.auth,
		);
	}
	async revokeAppInvite(appId: string, inviteId: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/team/invites/${inviteId}`,
			{
				method: "DELETE",
			},
			this.backend.auth,
		);
	}
	async acceptInvite(inviteId: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`user/invites/${inviteId}`,
			{
				method: "POST",
			},
			this.backend.auth,
		);
	}
	async rejectInvite(inviteId: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`user/invites/${inviteId}`,
			{
				method: "DELETE",
			},
			this.backend.auth,
		);
	}

	async inviteUser(
		appId: string,
		user_id: string,
		message: string,
	): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/team/invite`,
			{
				method: "PUT",
				body: JSON.stringify({
					sub: user_id,
					message: message,
				}),
			},
			this.backend.auth,
		);
	}

	async removeUser(appId: string, user_id: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/team/${user_id}`,
			{
				method: "DELETE",
			},
			this.backend.auth,
		);
	}

	async getAppConnections(appId: string): Promise<IAppConnectionsResponse> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		return await fetcher(
			this.backend.profile,
			`apps/${appId}/connections`,
			{
				method: "GET",
			},
			this.backend.auth,
		);
	}

	async addAppConnection(
		appId: string,
		sourceAppId: string,
		roleId: string,
	): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/connections`,
			{
				method: "POST",
				body: JSON.stringify({
					source_app_id: sourceAppId,
					role_id: roleId,
				}),
			},
			this.backend.auth,
		);
	}

	async requestAppConnection(
		appId: string,
		targetAppId: string,
		comment?: string,
	): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/connections/request`,
			{
				method: "PUT",
				body: JSON.stringify({
					target_app_id: targetAppId,
					comment: comment,
				}),
			},
			this.backend.auth,
		);
	}

	async acceptAppConnection(
		appId: string,
		connectionId: string,
		roleId: string,
	): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/connections/queue/${connectionId}`,
			{
				method: "POST",
				body: JSON.stringify({
					role_id: roleId,
				}),
			},
			this.backend.auth,
		);
	}

	async rejectAppConnection(
		appId: string,
		connectionId: string,
	): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/connections/queue/${connectionId}`,
			{
				method: "DELETE",
			},
			this.backend.auth,
		);
	}

	async updateAppConnectionRole(
		appId: string,
		connectionId: string,
		roleId: string,
	): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/connections/${connectionId}`,
			{
				method: "PUT",
				body: JSON.stringify({
					role_id: roleId,
				}),
			},
			this.backend.auth,
		);
	}

	async removeAppConnection(
		appId: string,
		connectionId: string,
	): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/connections/${connectionId}`,
			{
				method: "DELETE",
			},
			this.backend.auth,
		);
	}

	async getAccessibleApps(appId: string): Promise<IAccessibleApp[]> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		return await fetcher(
			this.backend.profile,
			`apps/${appId}/connections/accessible`,
			{
				method: "GET",
			},
			this.backend.auth,
		);
	}

	async getRemoteTables(appId: string, targetAppId: string): Promise<string[]> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		return await fetcher(
			this.backend.profile,
			`apps/${appId}/connections/${targetAppId}/tables`,
			{
				method: "GET",
			},
			this.backend.auth,
		);
	}

	async getRemoteEvents(
		appId: string,
		targetAppId: string,
	): Promise<IRemoteEvent[]> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		return await fetcher(
			this.backend.profile,
			`apps/${appId}/connections/${targetAppId}/events`,
			{
				method: "GET",
			},
			this.backend.auth,
		);
	}

	async getRemoteEventDetail(
		appId: string,
		targetAppId: string,
		eventId: string,
	): Promise<IRemoteEventDetail> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		return await fetcher(
			this.backend.profile,
			`apps/${appId}/connections/${targetAppId}/events/${eventId}/detail`,
			{
				method: "GET",
			},
			this.backend.auth,
		);
	}

	async getConnectionGraph(
		appId: string,
		days?: number,
	): Promise<IProcessGraphResponse> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		let url = `apps/${appId}/connections/graph`;
		if (days !== undefined) {
			url += `?days=${days}`;
		}

		return await fetcher(
			this.backend.profile,
			url,
			{
				method: "GET",
			},
			this.backend.auth,
		);
	}

	async getProcessCases(
		appId: string,
		days?: number,
	): Promise<IProcessCasesResponse> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		let url = `apps/${appId}/connections/cases`;
		if (days !== undefined) {
			url += `?days=${days}`;
		}

		return await fetcher(
			this.backend.profile,
			url,
			{
				method: "GET",
			},
			this.backend.auth,
		);
	}

	async getProcessCaseRuns(
		appId: string,
		caseId: string,
	): Promise<IProcessCaseDetailResponse> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		return await fetcher(
			this.backend.profile,
			`apps/${appId}/connections/cases/${caseId}`,
			{
				method: "GET",
			},
			this.backend.auth,
		);
	}

	async getProcessNotes(appId: string): Promise<IProcessNote[]> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		return await fetcher(
			this.backend.profile,
			`apps/${appId}/connections/notes`,
			{
				method: "GET",
			},
			this.backend.auth,
		);
	}

	async createProcessNote(
		appId: string,
		content: string,
	): Promise<IProcessNote> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		return await fetcher(
			this.backend.profile,
			`apps/${appId}/connections/notes`,
			{
				method: "PUT",
				body: JSON.stringify({ content }),
			},
			this.backend.auth,
		);
	}

	async updateProcessNote(
		appId: string,
		noteId: string,
		content: string,
	): Promise<IProcessNote> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		return await fetcher(
			this.backend.profile,
			`apps/${appId}/connections/notes/${noteId}`,
			{
				method: "PUT",
				body: JSON.stringify({ content }),
			},
			this.backend.auth,
		);
	}

	async deleteProcessNote(appId: string, noteId: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}

		await fetcher(
			this.backend.profile,
			`apps/${appId}/connections/notes/${noteId}`,
			{
				method: "DELETE",
			},
			this.backend.auth,
		);
	}

	async createGroup(
		appId: string,
		payload: ICreateGroupPayload,
	): Promise<IGroup> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}
		return await fetcher(
			this.backend.profile,
			`apps/${appId}/groups`,
			{ method: "POST", body: JSON.stringify(payload) },
			this.backend.auth,
		);
	}

	async listGroups(appId: string): Promise<IGroup[]> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}
		return await fetcher(
			this.backend.profile,
			`apps/${appId}/groups`,
			{ method: "GET" },
			this.backend.auth,
		);
	}

	async getGroup(appId: string, groupId: string): Promise<IGroup> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}
		return await fetcher(
			this.backend.profile,
			`apps/${appId}/groups/${groupId}`,
			{ method: "GET" },
			this.backend.auth,
		);
	}

	async updateGroup(
		appId: string,
		groupId: string,
		payload: IUpdateGroupPayload,
	): Promise<IGroup> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}
		return await fetcher(
			this.backend.profile,
			`apps/${appId}/groups/${groupId}`,
			{ method: "PUT", body: JSON.stringify(payload) },
			this.backend.auth,
		);
	}

	async deleteGroup(appId: string, groupId: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}
		await fetcher(
			this.backend.profile,
			`apps/${appId}/groups/${groupId}`,
			{ method: "DELETE" },
			this.backend.auth,
		);
	}

	async addGroupMember(
		appId: string,
		groupId: string,
		memberAppId: string,
	): Promise<IGroup> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}
		return await fetcher(
			this.backend.profile,
			`apps/${appId}/groups/${groupId}/members`,
			{ method: "POST", body: JSON.stringify({ member_app_id: memberAppId }) },
			this.backend.auth,
		);
	}

	async removeGroupMember(
		appId: string,
		groupId: string,
		memberAppId: string,
	): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}
		await fetcher(
			this.backend.profile,
			`apps/${appId}/groups/${groupId}/members/${memberAppId}`,
			{ method: "DELETE" },
			this.backend.auth,
		);
	}

	async listGroupRequests(appId: string): Promise<IGroupMembershipRequest[]> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}
		return await fetcher(
			this.backend.profile,
			`apps/${appId}/groups/requests`,
			{ method: "GET" },
			this.backend.auth,
		);
	}

	async acceptGroupRequest(appId: string, memberId: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}
		await fetcher(
			this.backend.profile,
			`apps/${appId}/groups/requests/${memberId}`,
			{ method: "POST" },
			this.backend.auth,
		);
	}

	async declineGroupRequest(appId: string, memberId: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}
		await fetcher(
			this.backend.profile,
			`apps/${appId}/groups/requests/${memberId}`,
			{ method: "DELETE" },
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
		// Suites exist only online — there is no Tauri command backing them, so
		// this deliberately has no offline branch.
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Suites require an online app");
		}

		const params = new URLSearchParams();
		params.set("group_id", groupId);
		params.set("item", item);
		params.set(
			"extension",
			file.name.includes(".") ? (file.name.split(".").pop() ?? "") : "",
		);
		params.set("language", language ?? "en");

		const { signed_url } = await fetcher<{ signed_url: string }>(
			this.backend.profile,
			`apps/${appId}/meta/media?${params}`,
			{ method: "PUT" },
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
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}
		return await fetcher(
			this.backend.profile,
			`apps/${appId}/groups/${groupId}/visibility`,
			{
				method: "PATCH",
				body: JSON.stringify({
					visibility: toWireVisibility(visibility),
					message,
				}),
			},
			this.backend.auth,
		);
	}

	async getGroupPublication(
		appId: string,
		groupId: string,
	): Promise<IGroupPublicationStatus> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}
		return await fetcher(
			this.backend.profile,
			`apps/${appId}/groups/${groupId}/publication`,
			{ method: "GET" },
			this.backend.auth,
		);
	}

	async leaveGroup(appId: string, groupId: string): Promise<void> {
		if (!this.backend.profile || !this.backend.auth) {
			throw new Error("Profile or auth context not available");
		}
		await fetcher(
			this.backend.profile,
			`apps/${appId}/groups/${groupId}/membership`,
			{ method: "DELETE" },
			this.backend.auth,
		);
	}
}
