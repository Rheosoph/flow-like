import type {
	IBotChat,
	IBotRoomLimits,
	IBotTeamState,
	ICreateTeamRequest,
	IOrgView,
	IPinnableApp,
	ITeam,
	ITeamBriefing,
	ITeamPreview,
	ITeamPreviewRequest,
	ITeamThread,
	IUpdateTeamRequest,
	TeamRole,
} from "@flow-like/flow-like-ui";
import type { DataClassification } from "@flow-like/flow-like-ui/lib/data-classification";
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

export class BotTeamState implements IBotTeamState {
	constructor(private readonly backend: TauriBackend) {}

	private session(): IAccountSession {
		const session = accountSession(this.backend);
		if (!session) throw new Error(NEEDS_ACCOUNT);
		return session;
	}

	async getOrg(): Promise<IOrgView> {
		const session = accountSession(this.backend);
		if (!session) {
			return {
				teams: [],
				bots: [],
				invitations: [],
				threads: [],
				reach: [],
				chats: [],
				classifications: [],
				apps: [],
			};
		}
		const response = await get<Partial<IOrgView>>(
			session.profile,
			TEAMS,
			session.auth,
		);
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

	async acceptLead(teamId: string, botId: string): Promise<ITeam> {
		const { profile, auth } = this.session();
		return expectTeam(
			await post<ITeam>(
				profile,
				`${botPath(teamId, botId)}/lead/accept`,
				undefined,
				auth,
			),
			`let bot ${botId} lead team ${teamId}`,
		);
	}

	async declineLead(teamId: string, botId: string): Promise<void> {
		const { profile, auth } = this.session();
		await post(
			profile,
			`${botPath(teamId, botId)}/lead/decline`,
			undefined,
			auth,
		);
	}

	async resignLead(teamId: string, botId: string): Promise<ITeam> {
		const { profile, auth } = this.session();
		return expectTeam(
			await post<ITeam>(
				profile,
				`${botPath(teamId, botId)}/lead/resign`,
				undefined,
				auth,
			),
			`step bot ${botId} down as lead of team ${teamId}`,
		);
	}

	async openThread(teamId: string): Promise<ITeamThread> {
		const { profile, auth } = this.session();
		return expectThread(
			await post<ITeamThread>(
				profile,
				`${teamPath(teamId)}/thread`,
				undefined,
				auth,
			),
			`open your thread with team ${teamId}`,
		);
	}

	async requestChat(botId: string, note?: string | null): Promise<IBotChat> {
		const { profile, auth } = this.session();
		return expectChat(
			await post<IBotChat>(
				profile,
				`${TEAMS}/chats`,
				{ bot_id: botId, note: note ?? undefined },
				auth,
			),
			`ask for a chat with bot ${botId}`,
		);
	}

	async acceptChat(
		botId: string,
		userId: string,
		limits?: IBotRoomLimits | null,
	): Promise<IBotChat> {
		const { profile, auth } = this.session();
		return expectChat(
			await post<IBotChat>(
				profile,
				`${chatPath(botId, userId)}/accept`,
				{ limits: limits ?? undefined },
				auth,
			),
			`approve a chat with bot ${botId}`,
		);
	}

	async declineChat(botId: string, userId: string): Promise<void> {
		const { profile, auth } = this.session();
		await post(profile, `${chatPath(botId, userId)}/decline`, undefined, auth);
	}

	async endChat(botId: string, userId: string): Promise<void> {
		const { profile, auth } = this.session();
		await del(profile, chatPath(botId, userId), undefined, auth);
	}

	async previewTeam(
		teamId: string,
		request: ITeamPreviewRequest,
	): Promise<ITeamPreview> {
		const { profile, auth } = this.session();
		const response = await post<Partial<ITeamPreview>>(
			profile,
			`${teamPath(teamId)}/preview`,
			request,
			auth,
		);
		return { people: list(response?.people), bots: list(response?.bots) };
	}

	async setClassification(
		orgId: string,
		userId: string,
		classification: DataClassification | null,
	): Promise<void> {
		const { profile, auth } = this.session();
		await put(
			profile,
			`${teamPath(orgId)}/classifications/${encodeURIComponent(userId)}`,
			{ classification },
			auth,
		);
	}

	async listPinnableApps(teamId: string): Promise<IPinnableApp[]> {
		const { profile, auth } = this.session();
		const response = await get<IPinnableApp[]>(
			profile,
			`${teamPath(teamId)}/apps/pinnable`,
			auth,
		);
		return list(response);
	}

	async pinApp(teamId: string, appId: string): Promise<ITeam> {
		const { profile, auth } = this.session();
		return expectTeam(
			await post<ITeam>(
				profile,
				`${teamPath(teamId)}/apps`,
				{ app_id: appId },
				auth,
			),
			`pin app ${appId} to team ${teamId}`,
		);
	}

	async approveAppPin(teamId: string, appId: string): Promise<ITeam> {
		const { profile, auth } = this.session();
		return expectTeam(
			await post<ITeam>(
				profile,
				`${appPinPath(teamId, appId)}/approve`,
				undefined,
				auth,
			),
			`approve app ${appId} for team ${teamId}`,
		);
	}

	async declineAppPin(teamId: string, appId: string): Promise<void> {
		const { profile, auth } = this.session();
		await post(
			profile,
			`${appPinPath(teamId, appId)}/decline`,
			undefined,
			auth,
		);
	}

	async unpinApp(teamId: string, appId: string): Promise<void> {
		const { profile, auth } = this.session();
		await del(profile, appPinPath(teamId, appId), undefined, auth);
	}
}
