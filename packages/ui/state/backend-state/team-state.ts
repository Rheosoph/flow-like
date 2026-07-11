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
	IUpdateGroupPayload,
} from "./types";

export interface ITeamState {
	createInviteLink(appId: string, name: string, maxUses: number): Promise<void>;
	getInviteLinks(appId: string): Promise<IInviteLink[]>;
	removeInviteLink(appId: string, linkId: string): Promise<void>;
	joinInviteLink(appId: string, token: string): Promise<void>;
	requestJoin(appId: string, comment: string): Promise<void>;
	getJoinRequests(
		appId: string,
		offset?: number,
		limit?: number,
	): Promise<IJoinRequest[]>;
	acceptJoinRequest(appId: string, requestId: string): Promise<void>;
	rejectJoinRequest(appId: string, requestId: string): Promise<void>;
	getTeam(appId: string, offset?: number, limit?: number): Promise<IMember[]>;
	getInvites(offset?: number, limit?: number): Promise<IInvite[]>;
	acceptInvite(inviteId: string): Promise<void>;
	rejectInvite(inviteId: string): Promise<void>;
	inviteUser(appId: string, user_id: string, message: string): Promise<void>;
	removeUser(appId: string, user_id: string): Promise<void>;
	getAppConnections(appId: string): Promise<IAppConnectionsResponse>;
	addAppConnection(
		appId: string,
		sourceAppId: string,
		roleId: string,
	): Promise<void>;
	requestAppConnection(
		appId: string,
		targetAppId: string,
		comment?: string,
	): Promise<void>;
	acceptAppConnection(
		appId: string,
		connectionId: string,
		roleId: string,
	): Promise<void>;
	rejectAppConnection(appId: string, connectionId: string): Promise<void>;
	updateAppConnectionRole(
		appId: string,
		connectionId: string,
		roleId: string,
	): Promise<void>;
	removeAppConnection(appId: string, connectionId: string): Promise<void>;
	getAccessibleApps(appId: string): Promise<IAccessibleApp[]>;
	getRemoteTables(appId: string, targetAppId: string): Promise<string[]>;
	getRemoteEvents(appId: string, targetAppId: string): Promise<IRemoteEvent[]>;
	getRemoteEventDetail(
		appId: string,
		targetAppId: string,
		eventId: string,
	): Promise<IRemoteEventDetail>;
	getConnectionGraph(
		appId: string,
		days?: number,
	): Promise<IProcessGraphResponse>;
	getProcessCases(appId: string, days?: number): Promise<IProcessCasesResponse>;
	getProcessCaseRuns(
		appId: string,
		caseId: string,
	): Promise<IProcessCaseDetailResponse>;
	getProcessNotes(appId: string): Promise<IProcessNote[]>;
	createProcessNote(appId: string, content: string): Promise<IProcessNote>;
	updateProcessNote(
		appId: string,
		noteId: string,
		content: string,
	): Promise<IProcessNote>;
	deleteProcessNote(appId: string, noteId: string): Promise<void>;
	// App groups (curated store "suites")
	createGroup(appId: string, payload: ICreateGroupPayload): Promise<IGroup>;
	listGroups(appId: string): Promise<IGroup[]>;
	getGroup(appId: string, groupId: string): Promise<IGroup>;
	updateGroup(
		appId: string,
		groupId: string,
		payload: IUpdateGroupPayload,
	): Promise<IGroup>;
	deleteGroup(appId: string, groupId: string): Promise<void>;
	addGroupMember(
		appId: string,
		groupId: string,
		memberAppId: string,
	): Promise<IGroup>;
	removeGroupMember(
		appId: string,
		groupId: string,
		memberAppId: string,
	): Promise<void>;
	listGroupRequests(appId: string): Promise<IGroupMembershipRequest[]>;
	acceptGroupRequest(appId: string, memberId: string): Promise<void>;
	declineGroupRequest(appId: string, memberId: string): Promise<void>;
}
