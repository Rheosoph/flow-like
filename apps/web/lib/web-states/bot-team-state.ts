import type {
	IBotChat,
	IBotRoomLimits,
	IBotTeamState,
	ICreateTeamRequest,
	IOrgView,
	IPinnableApp,
	ITeam,
	ITeamBriefing,
	ITeamThread,
	IUpdateTeamRequest,
	TeamRole,
} from "@flow-like/flow-like-ui";
import type { DataClassification } from "@flow-like/flow-like-ui/lib/data-classification";
import { isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
	apiPatch,
	apiPost,
	apiPut,
} from "./api-utils";

const TEAMS = "bot-teams";

function teamPath(teamId: string): string {
	return `${TEAMS}/${encodeURIComponent(teamId)}`;
}

function personPath(teamId: string, userId: string): string {
	return `${teamPath(teamId)}/people/${encodeURIComponent(userId)}`;
}

function botPath(teamId: string, botId: string): string {
	return `${teamPath(teamId)}/bots/${encodeURIComponent(botId)}`;
}

function appPinPath(teamId: string, appId: string): string {
	return `${teamPath(teamId)}/apps/${encodeURIComponent(appId)}`;
}

function chatPath(botId: string, userId: string): string {
	return `${TEAMS}/chats/${encodeURIComponent(botId)}/${encodeURIComponent(userId)}`;
}

function expectTeam(response: unknown, operation: string): ITeam {
	if (!isRecord(response) || typeof response.id !== "string") {
		throw new Error(`Unexpected response while trying to ${operation}`);
	}
	return response as unknown as ITeam;
}

function expectChat(response: unknown, operation: string): IBotChat {
	if (
		!isRecord(response) ||
		typeof response.bot_id !== "string" ||
		typeof response.user_id !== "string"
	) {
		throw new Error(`Unexpected response while trying to ${operation}`);
	}
	return response as unknown as IBotChat;
}

function expectThread(response: unknown, operation: string): ITeamThread {
	if (!isRecord(response) || typeof response.conversation_id !== "string") {
		throw new Error(`Unexpected response while trying to ${operation}`);
	}
	return response as unknown as ITeamThread;
}

function list<T>(value: T[] | undefined): T[] {
	return Array.isArray(value) ? value : [];
}

export class WebBotTeamState implements IBotTeamState {
	constructor(private readonly backend: WebBackendRef) {}

	async getOrg(): Promise<IOrgView> {
		const response = await apiGet<Partial<IOrgView>>(TEAMS, this.backend.auth);
		return {
			teams: list(response?.teams),
			bots: list(response?.bots),
			invitations: list(response?.invitations),
			threads: list(response?.threads),
			reach: list(response?.reach),
			chats: list(response?.chats),
			classifications: list(response?.classifications),
			apps: list(response?.apps),
		};
	}

	async createTeam(request: ICreateTeamRequest): Promise<ITeam> {
		return expectTeam(
			await apiPost<ITeam>(TEAMS, request, this.backend.auth),
			"create the team",
		);
	}

	async updateTeam(
		teamId: string,
		request: IUpdateTeamRequest,
	): Promise<ITeam> {
		return expectTeam(
			await apiPatch<ITeam>(teamPath(teamId), request, this.backend.auth),
			`update team ${teamId}`,
		);
	}

	async deleteTeam(teamId: string): Promise<void> {
		await apiDelete(teamPath(teamId), this.backend.auth);
	}

	async invitePeople(
		teamId: string,
		userIds: string[],
		role?: TeamRole,
	): Promise<ITeam> {
		return expectTeam(
			await apiPost<ITeam>(
				`${teamPath(teamId)}/people`,
				{ user_ids: userIds, role },
				this.backend.auth,
			),
			`invite people to team ${teamId}`,
		);
	}

	async removePerson(teamId: string, userId: string): Promise<void> {
		await apiDelete(personPath(teamId, userId), this.backend.auth);
	}

	async setPersonRole(
		teamId: string,
		userId: string,
		role: TeamRole,
	): Promise<ITeam> {
		return expectTeam(
			await apiPut<ITeam>(
				`${personPath(teamId, userId)}/role`,
				{ role },
				this.backend.auth,
			),
			`change a role in team ${teamId}`,
		);
	}

	async acceptTeamInvite(teamId: string): Promise<ITeam> {
		return expectTeam(
			await apiPost<ITeam>(
				`${teamPath(teamId)}/join`,
				undefined,
				this.backend.auth,
			),
			`join team ${teamId}`,
		);
	}

	async declineTeamInvite(teamId: string): Promise<void> {
		await apiPost(`${teamPath(teamId)}/decline`, undefined, this.backend.auth);
	}

	async addBot(
		teamId: string,
		botId: string,
		limits?: IBotRoomLimits | null,
	): Promise<ITeam> {
		return expectTeam(
			await apiPost<ITeam>(
				`${teamPath(teamId)}/bots`,
				{ bot_id: botId, limits: limits ?? undefined },
				this.backend.auth,
			),
			`add bot ${botId} to team ${teamId}`,
		);
	}

	async removeBot(teamId: string, botId: string): Promise<void> {
		await apiDelete(botPath(teamId, botId), this.backend.auth);
	}

	async acceptBotInvite(
		teamId: string,
		botId: string,
		limits?: IBotRoomLimits | null,
	): Promise<ITeam> {
		return expectTeam(
			await apiPost<ITeam>(
				`${botPath(teamId, botId)}/accept`,
				{ limits: limits ?? undefined },
				this.backend.auth,
			),
			`accept bot ${botId} into team ${teamId}`,
		);
	}

	async declineBotInvite(teamId: string, botId: string): Promise<void> {
		await apiPost(
			`${botPath(teamId, botId)}/decline`,
			undefined,
			this.backend.auth,
		);
	}

	async setDirectAgreement(
		teamId: string,
		botId: string,
		agreed: boolean,
	): Promise<ITeam> {
		return expectTeam(
			await apiPut<ITeam>(
				`${botPath(teamId, botId)}/direct`,
				{ agreed },
				this.backend.auth,
			),
			`set direct asks for bot ${botId}`,
		);
	}

	async getBriefing(botId: string): Promise<ITeamBriefing> {
		const response = await apiGet<ITeamBriefing>(
			`${TEAMS}/bots/${encodeURIComponent(botId)}/briefing`,
			this.backend.auth,
		);
		if (!isRecord(response) || typeof response.text !== "string") {
			throw new Error(
				`Unexpected response while reading the briefing of ${botId}`,
			);
		}
		return response;
	}

	async acceptLead(teamId: string, botId: string): Promise<ITeam> {
		return expectTeam(
			await apiPost<ITeam>(
				`${botPath(teamId, botId)}/lead/accept`,
				undefined,
				this.backend.auth,
			),
			`let bot ${botId} lead team ${teamId}`,
		);
	}

	async declineLead(teamId: string, botId: string): Promise<void> {
		await apiPost(
			`${botPath(teamId, botId)}/lead/decline`,
			undefined,
			this.backend.auth,
		);
	}

	async resignLead(teamId: string, botId: string): Promise<ITeam> {
		return expectTeam(
			await apiPost<ITeam>(
				`${botPath(teamId, botId)}/lead/resign`,
				undefined,
				this.backend.auth,
			),
			`step bot ${botId} down as lead of team ${teamId}`,
		);
	}

	async openThread(teamId: string): Promise<ITeamThread> {
		return expectThread(
			await apiPost<ITeamThread>(
				`${teamPath(teamId)}/thread`,
				undefined,
				this.backend.auth,
			),
			`open your thread with team ${teamId}`,
		);
	}

	async requestChat(botId: string, note?: string | null): Promise<IBotChat> {
		return expectChat(
			await apiPost<IBotChat>(
				`${TEAMS}/chats`,
				{ bot_id: botId, note: note ?? undefined },
				this.backend.auth,
			),
			`ask for a chat with bot ${botId}`,
		);
	}

	async acceptChat(
		botId: string,
		userId: string,
		limits?: IBotRoomLimits | null,
	): Promise<IBotChat> {
		return expectChat(
			await apiPost<IBotChat>(
				`${chatPath(botId, userId)}/accept`,
				{ limits: limits ?? undefined },
				this.backend.auth,
			),
			`approve a chat with bot ${botId}`,
		);
	}

	async declineChat(botId: string, userId: string): Promise<void> {
		await apiPost(
			`${chatPath(botId, userId)}/decline`,
			undefined,
			this.backend.auth,
		);
	}

	async endChat(botId: string, userId: string): Promise<void> {
		await apiDelete(chatPath(botId, userId), this.backend.auth);
	}

	async setClassification(
		orgId: string,
		userId: string,
		classification: DataClassification | null,
	): Promise<void> {
		await apiPut(
			`${teamPath(orgId)}/classifications/${encodeURIComponent(userId)}`,
			{ classification },
			this.backend.auth,
		);
	}

	async listPinnableApps(teamId: string): Promise<IPinnableApp[]> {
		const response = await apiGet<IPinnableApp[]>(
			`${teamPath(teamId)}/apps/pinnable`,
			this.backend.auth,
		);
		return list(response);
	}

	async pinApp(teamId: string, appId: string): Promise<ITeam> {
		return expectTeam(
			await apiPost<ITeam>(
				`${teamPath(teamId)}/apps`,
				{ app_id: appId },
				this.backend.auth,
			),
			`pin app ${appId} to team ${teamId}`,
		);
	}

	async approveAppPin(teamId: string, appId: string): Promise<ITeam> {
		return expectTeam(
			await apiPost<ITeam>(
				`${appPinPath(teamId, appId)}/approve`,
				undefined,
				this.backend.auth,
			),
			`approve app ${appId} for team ${teamId}`,
		);
	}

	async declineAppPin(teamId: string, appId: string): Promise<void> {
		await apiPost(
			`${appPinPath(teamId, appId)}/decline`,
			undefined,
			this.backend.auth,
		);
	}

	async unpinApp(teamId: string, appId: string): Promise<void> {
		await apiDelete(appPinPath(teamId, appId), this.backend.auth);
	}
}
