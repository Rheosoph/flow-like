import type { BotAppearance } from "../../components/bots/types";
import type { BotLocation, BotSystemKey } from "./bot-state";
import type { IBotRoomLimits } from "./inbox-state";

/** What a person may do in a team. Admins rename, set purpose, lead and place, invite and remove. */
export type TeamRole = "admin" | "member";

/** Invited and not yet accepted (by the person, or by the bot's owner), or in the team. */
export type TeamMemberStatus = "pending" | "active";

export interface ITeamPerson {
	user_id: string;
	role: TeamRole;
	status: TeamMemberStatus;
	invited_by?: string | null;
}

export interface ITeamBot {
	bot_id: string;
	owner_id: string;
	status: TeamMemberStatus;
	/** The bot's owner agreed to direct asks in this team. */
	direct_agreed: boolean;
	invited_by?: string | null;
}

export interface ITeam {
	id: string;
	name: string;
	/** What every bot in the team reads about its work. */
	purpose?: string | null;
	/** The team it sits in; null for a top-level team, which names its org. */
	parent_id?: string | null;
	/** One of the team's active bots. */
	lead_bot_id?: string | null;
	/** Teammates ask each other without the first-time Allow card once their owners agreed. */
	direct_asks: boolean;
	/** The team room: a group conversation of the team's active people and bots. */
	conversation_id?: string | null;
	people: ITeamPerson[];
	bots: ITeamBot[];
	created_at: string;
	updated_at: string;
}

/** A bot as every member of its org sees it: who it is and what it does, never its settings. */
export interface IOrgBot {
	id: string;
	owner_id: string;
	system?: BotSystemKey | null;
	/** Null on a built-in bot its owner never renamed. */
	name?: string | null;
	/** Its job description. */
	description?: string | null;
	appearance: BotAppearance;
	location: BotLocation;
	/** The bot's data classification, lowercase; null counts as internal. */
	classification?: string | null;
}

export type TeamInvitationKind = "person" | "bot" | "direct_asks";

/** One answer the reader owes a team. */
export interface ITeamInvitation {
	kind: TeamInvitationKind;
	team_id: string;
	team_name: string;
	/** The name of the team's top-level team. */
	org_name: string;
	/** The reader's bot, for `bot` and `direct_asks`. */
	bot_id?: string | null;
	invited_by?: string | null;
	created_at: string;
}

/** Every team of every org the reader belongs to, the bots in them, and what waits for the reader. */
export interface IOrgView {
	teams: ITeam[];
	bots: IOrgBot[];
	invitations: ITeamInvitation[];
	/** The viewer's own threads with teams, where their `#Team` requests went. */
	threads?: ITeamThread[];
}

/** A person's thread with a team: their `#Team` requests land there and the team's lead answers. */
export interface ITeamThread {
	team_id: string;
	conversation_id: string;
}

/** `body.team_request` on the copy of a `#Team` request in the person's thread. */
export interface ITeamRequestRef {
	team_id: string;
	origin_conversation_id: string;
	origin_message_id: string;
	/** The lead's answer is copied back into the origin, the person's own chat with a bot. */
	answer_here: boolean;
}

/** An activity entry's body: a lead's answer to a `#Team` request, copied into the chat it came from. */
export interface ITeamAnswerActivity {
	type: "team_answer";
	team_id: string;
	lead_id: string;
	thread_id: string;
	request_message_id: string;
	reply_message_id: string;
	text: string;
}

export interface ICreateTeamRequest {
	name: string;
	purpose?: string | null;
	/** Makes it a team inside this one; you must be an admin there. */
	parent_id?: string | null;
}

/** Omitted fields stay as they are; `null` clears a nullable field. */
export interface IUpdateTeamRequest {
	name?: string;
	purpose?: string | null;
	parent_id?: string | null;
	lead_bot_id?: string | null;
	direct_asks?: boolean;
}

/** The exact text a bot in a team reads about its team and org before every turn. */
export interface ITeamBriefing {
	bot_id: string;
	text: string;
}

/** Bot teams on the signed-in account's hub. Absent without a hub account. */
export interface IBotTeamState {
	getOrg(): Promise<IOrgView>;
	createTeam(request: ICreateTeamRequest): Promise<ITeam>;
	updateTeam(teamId: string, request: IUpdateTeamRequest): Promise<ITeam>;
	/** Teams inside it move up to its parent; the team room stays as a plain group. */
	deleteTeam(teamId: string): Promise<void>;
	/** Admins invite people; each accepts for themselves. */
	invitePeople(
		teamId: string,
		userIds: string[],
		role?: TeamRole,
	): Promise<ITeam>;
	/** An admin removes someone, or a person leaves (their own id). */
	removePerson(teamId: string, userId: string): Promise<void>;
	setPersonRole(teamId: string, userId: string, role: TeamRole): Promise<ITeam>;
	acceptTeamInvite(teamId: string): Promise<ITeam>;
	declineTeamInvite(teamId: string): Promise<void>;
	/**
	 * Your own bot joins at once (leaving its previous team); an admin's invite of a member's bot
	 * waits for that bot's owner.
	 */
	addBot(
		teamId: string,
		botId: string,
		limits?: IBotRoomLimits | null,
	): Promise<ITeam>;
	/** An admin or the bot's owner takes it out, or withdraws a pending invite. */
	removeBot(teamId: string, botId: string): Promise<void>;
	acceptBotInvite(
		teamId: string,
		botId: string,
		limits?: IBotRoomLimits | null,
	): Promise<ITeam>;
	declineBotInvite(teamId: string, botId: string): Promise<void>;
	/** The bot's owner agrees (or stops agreeing) to direct asks in the team. */
	setDirectAgreement(
		teamId: string,
		botId: string,
		agreed: boolean,
	): Promise<ITeam>;
	getBriefing(botId: string): Promise<ITeamBriefing>;
}
