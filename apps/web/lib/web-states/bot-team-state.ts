import type {
	IBotRoomLimits,
	IBotTeamState,
	ICreateTeamRequest,
	IOrgView,
	ITeam,
	ITeamBriefing,
	IUpdateTeamRequest,
	TeamRole,
} from "@flow-like/flow-like-ui";
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

function expectTeam(response: unknown, operation: string): ITeam {
	if (!isRecord(response) || typeof response.id !== "string") {
		throw new Error(`Unexpected response while trying to ${operation}`);
	}
	return response as unknown as ITeam;
}

export class WebBotTeamState implements IBotTeamState {
	constructor(private readonly backend: WebBackendRef) {}

	async getOrg(): Promise<IOrgView> {
		const response = await apiGet<Partial<IOrgView>>(TEAMS, this.backend.auth);
		return {
			teams: Array.isArray(response?.teams) ? response.teams : [],
			bots: Array.isArray(response?.bots) ? response.bots : [],
			invitations: Array.isArray(response?.invitations)
				? response.invitations
				: [],
			threads: Array.isArray(response?.threads) ? response.threads : [],
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
}
