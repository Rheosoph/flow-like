import type { BotAppearance } from "../../components/bots/types";
import type { DataClassification } from "../../lib/data-classification";
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
	/** The bot an admin asked to lead the team, until its owner answers; only admins and that owner see it. */
	lead_offer_bot_id?: string | null;
	/** Bots that take no turn in the team room while someone in it is classified below them. */
	gated_bots?: string[];
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
	/** What starts its work from outside the chats, and what it sends out. Older hubs leave it out. */
	inputs?: IOrgBotInputs;
}

/** Days `IOrgBotMail` counts mail over. */
export const ORG_MAIL_WINDOW_DAYS = 7;

export interface IOrgBotMail {
	address: string;
	/** Mails it received in the last `ORG_MAIL_WINDOW_DAYS` days. */
	received: number;
	/** Mails it sent in the last `ORG_MAIL_WINDOW_DAYS` days. */
	sent: number;
}

export interface IOrgBotWebhook {
	name: string;
	last_called_at?: string | null;
}

export type OrgCalendarProvider = "google" | "microsoft" | "caldav";

/** How work reaches a bot from outside its chats; never a webhook path, a secret or an account. */
export interface IOrgBotInputs {
	/** Its own mail address while the email ability is on. */
	mail?: IOrgBotMail | null;
	/** Its switched-on webhooks while the webhook ability is on. */
	webhooks: IOrgBotWebhook[];
	/** The calendar ability is on: it keeps its own calendar and its events wake it. */
	own_calendar: boolean;
	/** One provider per connected calendar it reads. */
	connected_calendars: OrgCalendarProvider[];
}

/**
 * `lead`: an admin asks the reader's bot to lead the team (everyone in the org can then talk to it).
 * `chat_request`: someone in the org (`user_id`) asks to talk with the reader's bot.
 */
export type TeamInvitationKind =
	| "person"
	| "bot"
	| "direct_asks"
	| "lead"
	| "chat_request";

/** One answer the reader owes a team. */
export interface ITeamInvitation {
	kind: TeamInvitationKind;
	team_id: string;
	team_name: string;
	/** The name of the team's top-level team. */
	org_name: string;
	/** The reader's bot, for every kind but `person`. */
	bot_id?: string | null;
	invited_by?: string | null;
	/** For `chat_request`: who asks. */
	user_id?: string | null;
	/** For `chat_request`: why they ask. */
	note?: string | null;
	created_at: string;
}

/** Characters of the note a person sends with a chat request. */
export const MAX_CHAT_NOTE_CHARS = 280;
/** Days a declined chat request waits before the person may ask again. */
export const CHAT_RETRY_DAYS = 7;

/**
 * Whether and where the reader can talk with a bot of their org:
 * `own` their bot; `unavailable` invited, not accepted yet; `classified` above the reader;
 * `team_room` a bot of the reader's team; `lead` another team's lead (the reader's thread with it);
 * `chat` an approved chat; `requested` waiting for the owner; `declined` ask again from `retry_at`;
 * `request` the reader may ask its owner for a chat.
 */
export type TalkReach =
	| "own"
	| "unavailable"
	| "classified"
	| "team_room"
	| "lead"
	| "chat"
	| "requested"
	| "declined"
	| "request";

export interface IBotReach {
	bot_id: string;
	reach: TalkReach;
	/** The team room, the reader's thread with the team, or the approved chat; null for a lead until the thread exists. */
	conversation_id?: string | null;
	/** For `declined`. */
	retry_at?: string | null;
}

export type BotChatStatus = "pending" | "active" | "declined" | "ended";

/** A person's chat with another owner's bot; members are the person, the bot and the owner, who watches. */
export interface IBotChat {
	bot_id: string;
	/** The person who asked. */
	user_id: string;
	/** The bot's owner, who decides and watches. */
	owner_id: string;
	status: BotChatStatus;
	/** Exists once approved. */
	conversation_id?: string | null;
	note?: string | null;
	requested_at: string;
	decided_at?: string | null;
}

/** A person's classification in an org, set by an admin of its top-level team. */
export interface IPersonClassification {
	/** The org's top-level team. */
	org_id: string;
	user_id: string;
	classification: DataClassification;
}

/** Every team of every org the reader belongs to, the bots in them, and what waits for the reader. */
export interface IOrgView {
	teams: ITeam[];
	bots: IOrgBot[];
	invitations: ITeamInvitation[];
	/** The viewer's own threads with teams, where their `#Team` requests went. */
	threads?: ITeamThread[];
	/** How the reader reaches each bot in `bots`. Older hubs leave it out. */
	reach?: IBotReach[];
	/** The reader's chats with other owners' bots, and other people's chats with the reader's bots. */
	chats?: IBotChat[];
	/** The reader's own classification per org, and everyone's in orgs whose top-level team they administer. Missing: internal. */
	classifications?: IPersonClassification[];
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
	/** The bot's owner accepts an admin's request that it lead the team. */
	acceptLead(teamId: string, botId: string): Promise<ITeam>;
	declineLead(teamId: string, botId: string): Promise<void>;
	/** The lead's owner steps it down; the team has no lead until an admin picks one. */
	resignLead(teamId: string, botId: string): Promise<ITeam>;
	/** Opens (or creates) the reader's thread with the team's lead. */
	openThread(teamId: string): Promise<ITeamThread>;
	/** Asks the bot's owner for a chat with the bot. */
	requestChat(botId: string, note?: string | null): Promise<IBotChat>;
	/** The bot's owner approves a chat request, with what the bot may do in it. */
	acceptChat(
		botId: string,
		userId: string,
		limits?: IBotRoomLimits | null,
	): Promise<IBotChat>;
	declineChat(botId: string, userId: string): Promise<void>;
	/** The person withdraws or ends the chat, or the owner ends it; the person keeps the history. */
	endChat(botId: string, userId: string): Promise<void>;
	/** An admin of the org's top-level team sets a person's classification; null makes it internal. */
	setClassification(
		orgId: string,
		userId: string,
		classification: DataClassification | null,
	): Promise<void>;
}
