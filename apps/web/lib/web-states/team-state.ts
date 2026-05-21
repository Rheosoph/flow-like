import type { ITeamState } from "@flow-like/flow-like-ui";
import type {
	IInvite,
	IInviteLink,
	IJoinRequest,
	IMember,
} from "@flow-like/flow-like-ui/state/backend-state/types";
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
	apiPost,
	apiPut,
} from "./api-utils";

export class WebTeamState implements ITeamState {
	constructor(private readonly backend: WebBackendRef) {}

	async createInviteLink(
		appId: string,
		name: string,
		maxUses: number,
	): Promise<void> {
		await apiPut(
			`apps/${appId}/team/link`,
			{ name, max_uses: maxUses },
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

	async acceptInvite(inviteId: string): Promise<void> {
		await apiPost(
			`user/invites/${inviteId}/accept`,
			undefined,
			this.backend.auth,
		);
	}

	async rejectInvite(inviteId: string): Promise<void> {
		await apiPost(
			`user/invites/${inviteId}/reject`,
			undefined,
			this.backend.auth,
		);
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
}
