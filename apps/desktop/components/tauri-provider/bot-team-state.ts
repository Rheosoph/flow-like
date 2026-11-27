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
import { del, get, patch, post, put } from "../../lib/api";
import {
	type IAccountSession,
	accountSession,
} from "../../lib/local-bots/account";
import type { TauriBackend } from "../tauri-provider";

const TEAMS = "bot-teams";
const NEEDS_ACCOUNT = "Bot teams need an account";

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

export class BotTeamState implements IBotTeamState {
	constructor(private readonly backend: TauriBackend) {}

	private session(): IAccountSession {
		const session = accountSession(this.backend);
		if (!session) throw new Error(NEEDS_ACCOUNT);
		return session;
	}

	async getOrg(): Promise<IOrgView> {
		const session = accountSession(this.backend);
		if (!session) return { teams: [], bots: [], invitations: [], threads: [] };
		const response = await get<Partial<IOrgView>>(
			session.profile,
			TEAMS,
			session.auth,
		);
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
		const { profile, auth } = this.session();
		return expectTeam(
			await post<ITeam>(profile, TEAMS, request, auth),
			"create the team",
		);
	}

	async updateTeam(
		teamId: string,
		request: IUpdateTeamRequest,
	): Promise<ITeam> {
		const { profile, auth } = this.session();
		return expectTeam(
			await patch<ITeam>(profile, teamPath(teamId), request, auth),
			`update team ${teamId}`,
		);
	}

	async deleteTeam(teamId: string): Promise<void> {
		const { profile, auth } = this.session();
		await del(profile, teamPath(teamId), undefined, auth);
	}

	async invitePeople(
		teamId: string,
		userIds: string[],
		role?: TeamRole,
	): Promise<ITeam> {
		const { profile, auth } = this.session();
		return expectTeam(
			await post<ITeam>(
				profile,
				`${teamPath(teamId)}/people`,
				{ user_ids: userIds, role },
				auth,
			),
			`invite people to team ${teamId}`,
		);
	}

	async removePerson(teamId: string, userId: string): Promise<void> {
		const { profile, auth } = this.session();
		await del(profile, personPath(teamId, userId), undefined, auth);
	}

	async setPersonRole(
		teamId: string,
		userId: string,
		role: TeamRole,
	): Promise<ITeam> {
		const { profile, auth } = this.session();
		return expectTeam(
			await put<ITeam>(
				profile,
				`${personPath(teamId, userId)}/role`,
				{ role },
				auth,
			),
			`change a role in team ${teamId}`,
		);
	}

	async acceptTeamInvite(teamId: string): Promise<ITeam> {
		const { profile, auth } = this.session();
		return expectTeam(
			await post<ITeam>(profile, `${teamPath(teamId)}/join`, undefined, auth),
			`join team ${teamId}`,
		);
	}

	async declineTeamInvite(teamId: string): Promise<void> {
		const { profile, auth } = this.session();
		await post(profile, `${teamPath(teamId)}/decline`, undefined, auth);
	}

	async addBot(
		teamId: string,
		botId: string,
		limits?: IBotRoomLimits | null,
	): Promise<ITeam> {
		const { profile, auth } = this.session();
		return expectTeam(
			await post<ITeam>(
				profile,
				`${teamPath(teamId)}/bots`,
				{ bot_id: botId, limits: limits ?? undefined },
				auth,
			),
			`add bot ${botId} to team ${teamId}`,
		);
	}

	async removeBot(teamId: string, botId: string): Promise<void> {
		const { profile, auth } = this.session();
		await del(profile, botPath(teamId, botId), undefined, auth);
	}

	async acceptBotInvite(
		teamId: string,
		botId: string,
		limits?: IBotRoomLimits | null,
	): Promise<ITeam> {
		const { profile, auth } = this.session();
		return expectTeam(
			await post<ITeam>(
				profile,
				`${botPath(teamId, botId)}/accept`,
				{ limits: limits ?? undefined },
				auth,
			),
			`accept bot ${botId} into team ${teamId}`,
		);
	}

	async declineBotInvite(teamId: string, botId: string): Promise<void> {
		const { profile, auth } = this.session();
		await post(profile, `${botPath(teamId, botId)}/decline`, undefined, auth);
	}

	async setDirectAgreement(
		teamId: string,
		botId: string,
		agreed: boolean,
	): Promise<ITeam> {
		const { profile, auth } = this.session();
		return expectTeam(
			await put<ITeam>(
				profile,
				`${botPath(teamId, botId)}/direct`,
				{ agreed },
				auth,
			),
			`set direct asks for bot ${botId}`,
		);
	}

	async getBriefing(botId: string): Promise<ITeamBriefing> {
		const { profile, auth } = this.session();
		const response = await get<ITeamBriefing>(
			profile,
			`${TEAMS}/bots/${encodeURIComponent(botId)}/briefing`,
			auth,
		);
		if (!isRecord(response) || typeof response.text !== "string") {
			throw new Error(
				`Unexpected response while reading the briefing of ${botId}`,
			);
		}
		return response;
	}
}
