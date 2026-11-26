import {
	type OrgTeamNode,
	activeTeamOf,
	flattenForest,
	orgBot,
	orgForest,
	teamTag,
} from "../../lib/bot-teams/org-model";
import type {
	IOrgBot,
	IOrgView,
	ITeam,
} from "../../state/backend-state/bot-team-state";
import type { IConversation } from "../../state/backend-state/inbox-state";
import type { CrewMember } from "./crew-grid";
import { directPartnerId, isSelfPrincipal, matchesSearch } from "./crew-model";

export type ChatFilter = "all" | "unread" | "pinned";

interface RowBase {
	key: string;
	name: string;
	snippet: string;
	activityAt: string | null;
	unread: number;
	pinned: boolean;
}

/** One of the reader's bots, with its direct chat's newest line when there is one. */
export interface BotRow extends RowBase {
	kind: "bot";
	member: CrewMember;
	conversation: IConversation | null;
	lead: boolean;
}

/** A colleague's bot in a team: it opens the team room, where it takes part. */
export interface ColleagueBotRow extends RowBase {
	kind: "colleague";
	bot: IOrgBot;
	team: ITeam;
	lead: boolean;
	pending: boolean;
	ownerId: string;
	ownerName: string;
}

export interface ConversationRow extends RowBase {
	kind: "conversation";
	conversation: IConversation;
	team: ITeam | null;
	tag: string | null;
}

export type ChatRow = BotRow | ColleagueBotRow | ConversationRow;

export type ChatGroupKind = "pinned" | "team" | "bots" | "rooms" | "people";

export interface ChatGroup {
	key: string;
	kind: ChatGroupKind;
	team: ITeam | null;
	depth: number;
	rows: ChatRow[];
	unread: number;
}

export interface ChatListInput {
	org: IOrgView;
	me?: string | null;
	conversations: readonly IConversation[];
	crew: readonly CrewMember[];
	filter: ChatFilter;
	query: string;
	title: (conversation: IConversation) => string;
	snippet: (conversation: IConversation) => string;
	orgBotName: (bot: IOrgBot) => string;
	ownerName: (ownerId: string) => string;
}

const byActivity = (a: IConversation, b: IConversation) =>
	Date.parse(b.last_activity_at) - Date.parse(a.last_activity_at);

function conversationRow(
	input: ChatListInput,
	conversation: IConversation,
	team: ITeam | null = null,
): ConversationRow {
	return {
		kind: "conversation",
		key: `conversation:${conversation.id}`,
		name: input.title(conversation),
		snippet: input.snippet(conversation),
		activityAt: conversation.last_activity_at,
		unread: conversation.unread_count,
		pinned: Boolean(conversation.pinned_at),
		conversation,
		team,
		tag: team ? teamTag(team.name) : null,
	};
}

function botRow(
	input: ChatListInput,
	member: CrewMember,
	conversation: IConversation | null,
	lead: boolean,
): BotRow {
	return {
		kind: "bot",
		key: `bot:${member.key}`,
		name: member.name,
		snippet: conversation ? input.snippet(conversation) : member.description,
		activityAt: conversation?.last_activity_at ?? null,
		unread: conversation?.unread_count ?? 0,
		pinned: Boolean(conversation?.pinned_at),
		member,
		conversation,
		lead,
	};
}

function sortTeamBots<T extends { lead: boolean; name: string }>(
	rows: T[],
): T[] {
	return rows.sort(
		(a, b) => Number(b.lead) - Number(a.lead) || a.name.localeCompare(b.name),
	);
}

function totalUnread(rows: readonly ChatRow[]): number {
	return rows.reduce((sum, row) => sum + row.unread, 0);
}

/** Every bot id the crew holds and its own direct conversation, which belongs on the bot's row. */
function directChatsOfCrew(
	crew: readonly CrewMember[],
	conversations: readonly IConversation[],
	me?: string | null,
): Map<string, IConversation> {
	const own = new Set(crew.map((member) => member.bot?.id ?? member.key));
	const chats = new Map<string, IConversation>();
	for (const conversation of conversations) {
		const partner = directPartnerId(conversation, me);
		if (partner && own.has(partner) && !chats.has(partner)) {
			chats.set(partner, conversation);
		}
	}
	return chats;
}

function teamGroup(
	input: ChatListInput,
	node: OrgTeamNode,
	crewById: ReadonlyMap<string, CrewMember>,
	directChats: ReadonlyMap<string, IConversation>,
	byId: ReadonlyMap<string, IConversation>,
): ChatGroup {
	const { team } = node;
	const room = team.conversation_id
		? byId.get(team.conversation_id)
		: undefined;
	const rows: ChatRow[] = room ? [conversationRow(input, room, team)] : [];
	const members: (BotRow | ColleagueBotRow)[] = [];
	const pending: ColleagueBotRow[] = [];
	for (const entry of team.bots) {
		const own = crewById.get(entry.bot_id);
		const lead = team.lead_bot_id === entry.bot_id;
		if (own) {
			if (entry.status === "active") {
				members.push(
					botRow(input, own, directChats.get(entry.bot_id) ?? null, lead),
				);
			}
			continue;
		}
		const bot = orgBot(input.org, entry.bot_id);
		if (!bot || isSelfPrincipal(bot.owner_id, input.me)) continue;
		const row: ColleagueBotRow = {
			kind: "colleague",
			key: `colleague:${team.id}:${bot.id}`,
			name: input.orgBotName(bot),
			snippet: bot.description?.trim() ?? "",
			activityAt: null,
			unread: 0,
			pinned: false,
			bot,
			team,
			lead,
			pending: entry.status === "pending",
			ownerId: bot.owner_id,
			ownerName: input.ownerName(bot.owner_id),
		};
		(row.pending ? pending : members).push(row);
	}
	rows.push(...sortTeamBots(members), ...pending);
	return {
		key: `team:${team.id}`,
		kind: "team",
		team,
		depth: node.depth,
		rows,
		unread: totalUnread(rows),
	};
}

function keepRow(
	row: ChatRow,
	input: ChatListInput,
	group: ChatGroup,
): boolean {
	if (input.filter === "unread" && row.unread === 0) return false;
	if (input.filter === "pinned" && !row.pinned) return false;
	if (!input.query) return true;
	return matchesSearch(input.query, [
		row.name,
		row.snippet,
		group.team?.name,
		row.kind === "colleague" ? row.ownerName : null,
	]);
}

function filtered(input: ChatListInput, group: ChatGroup): ChatGroup | null {
	const plain = input.filter === "all" && !input.query;
	if (plain) return group.rows.length > 0 ? group : null;
	const teamMatches =
		group.team !== null &&
		input.filter === "all" &&
		matchesSearch(input.query, [group.team.name]);
	const rows = teamMatches
		? group.rows
		: group.rows.filter((row) => keepRow(row, input, group));
	return rows.length === 0
		? null
		: { ...group, rows, unread: totalUnread(rows) };
}

/** Pinned, then each team in org order, then your bots, rooms and people. */
export function buildChatGroups(input: ChatListInput): ChatGroup[] {
	const { org, conversations, crew, me } = input;
	const byId = new Map(conversations.map((item) => [item.id, item]));
	const crewById = new Map(
		crew.map((member) => [member.bot?.id ?? member.key, member]),
	);
	const directChats = directChatsOfCrew(crew, conversations, me);
	const directIds = new Set([...directChats.values()].map((item) => item.id));
	const teamRoomIds = new Set(
		org.teams.flatMap((team) =>
			team.conversation_id ? [team.conversation_id] : [],
		),
	);

	const loose = conversations
		.filter((item) => !directIds.has(item.id) && !teamRoomIds.has(item.id))
		.sort(byActivity);
	const pinned = loose
		.filter((item) => item.pinned_at)
		.sort(
			(a, b) => Date.parse(b.pinned_at ?? "") - Date.parse(a.pinned_at ?? ""),
		);
	const unpinned = loose.filter((item) => !item.pinned_at);
	const groupOf = (
		kind: ChatGroupKind,
		items: readonly IConversation[],
	): ChatGroup => {
		const rows = items.map((item) => conversationRow(input, item));
		return {
			key: kind,
			kind,
			team: null,
			depth: 0,
			rows,
			unread: totalUnread(rows),
		};
	};

	const teams = flattenForest(orgForest(org)).map((node) =>
		teamGroup(input, node, crewById, directChats, byId),
	);
	const solo = crew.filter(
		(member) => !activeTeamOf(org, member.bot?.id ?? member.key),
	);
	const soloRows = solo.map((member) =>
		botRow(
			input,
			member,
			directChats.get(member.bot?.id ?? member.key) ?? null,
			false,
		),
	);
	const yourBots: ChatGroup = {
		key: "bots",
		kind: "bots",
		team: null,
		depth: 0,
		rows: soloRows,
		unread: totalUnread(soloRows),
	};

	return [
		groupOf("pinned", pinned),
		...teams,
		yourBots,
		groupOf(
			"rooms",
			unpinned.filter((item) => item.kind === "group"),
		),
		groupOf(
			"people",
			unpinned.filter((item) => item.kind !== "group"),
		),
	].flatMap((group) => {
		const kept = filtered(input, group);
		return kept ? [kept] : [];
	});
}

export interface TeamRoom {
	team: ITeam;
	conversation: IConversation;
}

/** The team rooms the reader can open, in org order. */
export function teamRooms(
	org: IOrgView,
	conversations: readonly IConversation[],
): TeamRoom[] {
	const byId = new Map(conversations.map((item) => [item.id, item]));
	return flattenForest(orgForest(org)).flatMap(({ team }) => {
		const conversation = team.conversation_id
			? byId.get(team.conversation_id)
			: undefined;
		return conversation ? [{ team, conversation }] : [];
	});
}
