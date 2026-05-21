import type { ITeamState } from "@flow-like/flow-like-ui";
import type {
	IInvite,
	IInviteLink,
	IJoinRequest,
	IMember,
} from "@flow-like/flow-like-ui/state/backend-state/types";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

export class TeamState implements ITeamState {
	constructor(private readonly backend: TauriBackend) {}
	async createInviteLink(
		appId: string,
		name: string,
		maxUses: number,
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

		offset = offset ?? 0;
		if (limit) {
			url += `?offset=${offset}&limit=${limit}`;
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
		offset = offset ?? 0;
		limit = limit ?? 20;
		if (limit) {
			url += `?offset=${offset}&limit=${limit}`;
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

		let url = `user/invites`;
		offset = offset ?? 0;
		limit = limit ?? 20;
		if (limit) {
			url += `?offset=${offset}&limit=${limit}`;
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
}
