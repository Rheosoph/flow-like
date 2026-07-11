import type {
	IAccessibleApp,
	IAppConnectionsResponse,
	ICreateGroupPayload,
	IGroup,
	IGroupMembershipRequest,
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
	ITeamState,
	IUpdateGroupPayload,
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
	getAppConnections(appId: string): Promise<IAppConnectionsResponse> {
		throw new Error("Method not implemented.");
	}
	addAppConnection(
		appId: string,
		sourceAppId: string,
		roleId: string,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	requestAppConnection(
		appId: string,
		targetAppId: string,
		comment?: string,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	acceptAppConnection(
		appId: string,
		connectionId: string,
		roleId: string,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	rejectAppConnection(appId: string, connectionId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	updateAppConnectionRole(
		appId: string,
		connectionId: string,
		roleId: string,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	removeAppConnection(appId: string, connectionId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	getAccessibleApps(appId: string): Promise<IAccessibleApp[]> {
		throw new Error("Method not implemented.");
	}
	getRemoteTables(appId: string, targetAppId: string): Promise<string[]> {
		throw new Error("Method not implemented.");
	}
	getRemoteEvents(appId: string, targetAppId: string): Promise<IRemoteEvent[]> {
		throw new Error("Method not implemented.");
	}
	getRemoteEventDetail(
		appId: string,
		targetAppId: string,
		eventId: string,
	): Promise<IRemoteEventDetail> {
		throw new Error("Method not implemented.");
	}
	getConnectionGraph(
		appId: string,
		days?: number,
	): Promise<IProcessGraphResponse> {
		throw new Error("Method not implemented.");
	}
	getProcessCases(
		appId: string,
		days?: number,
	): Promise<IProcessCasesResponse> {
		throw new Error("Method not implemented.");
	}
	getProcessCaseRuns(
		appId: string,
		caseId: string,
	): Promise<IProcessCaseDetailResponse> {
		throw new Error("Method not implemented.");
	}
	getProcessNotes(appId: string): Promise<IProcessNote[]> {
		throw new Error("Method not implemented.");
	}
	createProcessNote(appId: string, content: string): Promise<IProcessNote> {
		throw new Error("Method not implemented.");
	}
	updateProcessNote(
		appId: string,
		noteId: string,
		content: string,
	): Promise<IProcessNote> {
		throw new Error("Method not implemented.");
	}
	deleteProcessNote(appId: string, noteId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	createGroup(appId: string, payload: ICreateGroupPayload): Promise<IGroup> {
		throw new Error("Method not implemented.");
	}
	listGroups(appId: string): Promise<IGroup[]> {
		throw new Error("Method not implemented.");
	}
	getGroup(appId: string, groupId: string): Promise<IGroup> {
		throw new Error("Method not implemented.");
	}
	updateGroup(
		appId: string,
		groupId: string,
		payload: IUpdateGroupPayload,
	): Promise<IGroup> {
		throw new Error("Method not implemented.");
	}
	deleteGroup(appId: string, groupId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	addGroupMember(
		appId: string,
		groupId: string,
		memberAppId: string,
	): Promise<IGroup> {
		throw new Error("Method not implemented.");
	}
	removeGroupMember(
		appId: string,
		groupId: string,
		memberAppId: string,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	listGroupRequests(appId: string): Promise<IGroupMembershipRequest[]> {
		throw new Error("Method not implemented.");
	}
	acceptGroupRequest(appId: string, memberId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	declineGroupRequest(appId: string, memberId: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
}
