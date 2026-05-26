import type {
	IInvite,
	IInviteLink,
	IJoinRequest,
	IMember,
	ITeamState,
} from "@flow-like/flow-like-ui";

export class EmptyTeamState implements ITeamState {
	createInviteLink(
		appId: string,
		name: string,
		maxUses: number,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	getInviteLinks(appId: string): Promise<IInviteLink[]> {
		throw new Error("Method not implemented.");
	}
	removeInviteLink(appId: string, linkId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	joinInviteLink(appId: string, token: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	requestJoin(appId: string, comment: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	getJoinRequests(
		appId: string,
		offset?: number,
		limit?: number,
	): Promise<IJoinRequest[]> {
		throw new Error("Method not implemented.");
	}
	acceptJoinRequest(appId: string, requestId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	rejectJoinRequest(appId: string, requestId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	getTeam(appId: string, offset?: number, limit?: number): Promise<IMember[]> {
		throw new Error("Method not implemented.");
	}
	getInvites(offset?: number, limit?: number): Promise<IInvite[]> {
		throw new Error("Method not implemented.");
	}
	acceptInvite(inviteId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	rejectInvite(inviteId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	inviteUser(appId: string, user_id: string, message: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	removeUser(appId: string, user_id: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
}
