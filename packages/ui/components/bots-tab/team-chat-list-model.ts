import {
	type OrgTeamNode,
	activeTeamOf,
	flattenForest,
	isTeamMember,
	orgBot,
	orgForest,
	teamById,
	teamTag,
} from "../../lib/bot-teams/org-model";
import type {
	IOrgBot,
	IOrgView,
	ITeam,
	ITeamThread,
} from "../../state/backend-state/bot-team-state";
import type { IConversation } from "../../state/backend-state/inbox-state";
import type { CrewMember } from "./crew-grid";
import {
	directPartnerId,
	isSelfPrincipal,
	matchesSearch,
	unreadOf,
} from "./crew-model";

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
	/** The reader's own thread with the team, where their `#Team` requests go. */
	thread?: boolean;
	/** The reader's approved chat with another owner's bot. */
	botChat?: { bot: IOrgBot; ownerName: string };
	/** Someone else's conversation the reader's bot is in; the reader only watches. */
	watching?: boolean;
}

/** A teammate the reader has no direct chat with yet; opening the row starts one. */
export interface PersonRow extends RowBase {
	kind: "person";
	userId: string;
	team: ITeam;
}

export type ChatRow = BotRow | ColleagueBotRow | ConversationRow | PersonRow;

export type ChatGroupKind =
	| "pinned"
	| "team"
	| "bots"
	| "colleagues"
	| "rooms"
	| "people"
	| "watching";

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
	/** What a team's thread of the reader is called inside the team's group. */
	requestsLabel?: string;
	snippet: (conversation: IConversation) => string;
	orgBotName: (bot: IOrgBot) => string;
	ownerName: (ownerId: string) => string;
}

const byActivity = (a: IConversation, b: IConversation) =>
	Date.parse(b.last_activity_at) - Date.parse(a.last_activity_at);

const rowsByActivity = (a: ConversationRow, b: ConversationRow) =>
	byActivity(a.conversation, b.conversation);

const pinnedAt = (row: ChatRow) =>
	row.kind === "bot" || row.kind === "conversation"
		? Date.parse(row.conversation?.pinned_at ?? "")
		: 0;

const crewId = (member: CrewMember) => member.bot?.id ?? member.key;

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
		unread: unreadOf(conversation),
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
		unread: conversation ? unreadOf(conversation) : 0,
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
	const own = new Set(crew.map(crewId));
	const chats = new Map<string, IConversation>();
	for (const conversation of conversations) {
		const partner = directPartnerId(conversation, me);
		if (partner && own.has(partner) && !chats.has(partner)) {
			chats.set(partner, conversation);
		}
	}
	return chats;
}

/** Each teammate's team: the most specific one the reader is an active member of with them. */
function teammateTeams(
	nodes: readonly OrgTeamNode[],
	me?: string | null,
): Map<string, ITeam> {
	const home = new Map<string, OrgTeamNode>();
	for (const node of nodes) {
		if (!isTeamMember(node.team, me)) continue;
		for (const person of node.team.people) {
			if (person.status !== "active" || isSelfPrincipal(person.user_id, me))
				continue;
			const current = home.get(person.user_id);
			if (!current || node.depth > current.depth)
				home.set(person.user_id, node);
		}
	}
	return new Map([...home].map(([userId, node]) => [userId, node.team]));
}

/** The person on the other side of a direct chat, when it is a person and not a bot. */
function personPartner(
	conversation: IConversation,
	me?: string | null,
): string | undefined {
	const partner = directPartnerId(conversation, me);
	const kind = conversation.members.find(
		(member) => member.principal_id === partner,
	)?.kind;
	return kind === "user" ? partner : undefined;
}

/**
 * The team rows of the reader's teammates: their direct chats, newest first, then the teammates
 * without one by name.
 */
function teammateRows(
	input: ChatListInput,
	teammates: ReadonlyMap<string, ITeam>,
	chats: ReadonlyMap<string, IConversation>,
): Map<string, ChatRow[]> {
	const byTeam = new Map<
		string,
		{ chats: IConversation[]; people: PersonRow[] }
	>();
	for (const [userId, team] of teammates) {
		const entry = byTeam.get(team.id) ?? { chats: [], people: [] };
		byTeam.set(team.id, entry);
		const chat = chats.get(userId);
		if (chat) {
			entry.chats.push(chat);
			continue;
		}
		entry.people.push({
			kind: "person",
			key: `person:${team.id}:${userId}`,
			name: input.ownerName(userId),
			snippet: "",
			activityAt: null,
			unread: 0,
			pinned: false,
			userId,
			team,
		});
	}
	return new Map(
		[...byTeam].map(([teamId, entry]) => [
			teamId,
			[
				...entry.chats
					.sort(byActivity)
					.map((chat) => conversationRow(input, chat)),
				...entry.people.sort((a, b) => a.name.localeCompare(b.name)),
			],
		]),
	);
}

interface MyChats {
	active: ReadonlyMap<string, IConversation>;
	waiting: ReadonlySet<string>;
}

/** The reader's approved chats that are in the list, and the bots they still wait on. */
function myChats(
	org: IOrgView,
	me: string | null | undefined,
	byId: ReadonlyMap<string, IConversation>,
): MyChats {
	const active = new Map<string, IConversation>();
	const waiting = new Set<string>();
	for (const chat of org.chats ?? []) {
		if (!isSelfPrincipal(chat.user_id, me)) continue;
		if (chat.status === "pending") waiting.add(chat.bot_id);
		const conversation = chat.conversation_id
			? byId.get(chat.conversation_id)
			: undefined;
		if (chat.status === "active" && conversation)
			active.set(chat.bot_id, conversation);
	}
	return { active, waiting };
}

function botChatRow(
	input: ChatListInput,
	bot: IOrgBot,
	conversation: IConversation,
): ConversationRow {
	return {
		...conversationRow(input, conversation),
		name: input.orgBotName(bot),
		botChat: { bot, ownerName: input.ownerName(bot.owner_id) },
	};
}

function threadsOf(org: IOrgView, teamId: string): ITeamThread[] {
	return (org.threads ?? []).filter((thread) => thread.team_id === teamId);
}

function teamGroup(
	input: ChatListInput,
	node: OrgTeamNode,
	crewById: ReadonlyMap<string, CrewMember>,
	directChats: ReadonlyMap<string, IConversation>,
	byId: ReadonlyMap<string, IConversation>,
	teammates: readonly ChatRow[],
	mine: MyChats,
): ChatGroup {
	const { team } = node;
	const room = team.conversation_id
		? byId.get(team.conversation_id)
		: undefined;
	const rows: ChatRow[] = room ? [conversationRow(input, room, team)] : [];
	for (const thread of threadsOf(input.org, team.id)) {
		const conversation = byId.get(thread.conversation_id);
		if (!conversation) continue;
		rows.push({
			...conversationRow(input, conversation),
			name: input.requestsLabel ?? input.title(conversation),
			thread: true,
		});
	}
	const members: (BotRow | ColleagueBotRow)[] = [];
	const pending: ColleagueBotRow[] = [];
	const chats: ConversationRow[] = [];
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
		const chat = mine.active.get(bot.id);
		if (chat) {
			chats.push(botChatRow(input, bot, chat));
			continue;
		}
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
			pending: entry.status === "pending" || mine.waiting.has(bot.id),
			ownerId: bot.owner_id,
			ownerName: input.ownerName(bot.owner_id),
		};
		(row.pending ? pending : members).push(row);
	}
	rows.push(
		...sortTeamBots(members),
		...chats.sort(rowsByActivity),
		...pending,
		...teammates,
	);
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

/** The reader's bots whose direct chat is pinned. */
function pinnedBotRows(
	input: ChatListInput,
	directChats: ReadonlyMap<string, IConversation>,
): BotRow[] {
	return input.crew.flatMap((member) => {
		const id = crewId(member);
		const chat = directChats.get(id);
		if (!chat?.pinned_at) return [];
		const lead = activeTeamOf(input.org, id)?.lead_bot_id === id;
		return [botRow(input, member, chat, lead)];
	});
}

/**
 * When filtering or searching, a chat that Pinned and its team both hold shows once, in Pinned.
 */
function withoutRepeats(groups: readonly ChatGroup[]): ChatGroup[] {
	const seen = new Set<string>();
	return groups.flatMap((group) => {
		const rows = group.rows.filter((row) => {
			if (seen.has(row.key)) return false;
			seen.add(row.key);
			return true;
		});
		return rows.length === 0
			? []
			: [{ ...group, rows, unread: totalUnread(rows) }];
	});
}

function watchingName(input: ChatListInput, conversation: IConversation) {
	const chat = (input.org.chats ?? []).find(
		(item) => item.conversation_id === conversation.id,
	);
	const bot = chat ? orgBot(input.org, chat.bot_id) : undefined;
	return chat && bot
		? `${input.ownerName(chat.user_id)} · ${input.orgBotName(bot)}`
		: input.title(conversation);
}

/** Conversations of the reader's bots that the reader only watches, newest first, never unread. */
function watchingRows(
	input: ChatListInput,
	conversations: readonly IConversation[],
): ChatRow[] {
	return [...conversations].sort(byActivity).map((conversation) => ({
		...conversationRow(input, conversation),
		name: watchingName(input, conversation),
		unread: 0,
		pinned: false,
		watching: true,
	}));
}

/**
 * Pinned, then each team in org order, then your bots, rooms and people. A teammate's direct chat
 * sits in their team rather than People. Teams keep their pinned members, so Pinned repeats them;
 * everything else pinned leaves its group for Pinned.
 */
export function buildChatGroups(input: ChatListInput): ChatGroup[] {
	const { org, crew, me } = input;
	const conversations = input.conversations.filter((item) => !item.watching);
	const watching = input.conversations.filter((item) => item.watching);
	const byId = new Map(conversations.map((item) => [item.id, item]));
	const crewById = new Map(crew.map((member) => [crewId(member), member]));
	const directChats = directChatsOfCrew(crew, conversations, me);
	const directIds = new Set([...directChats.values()].map((item) => item.id));
	const teamRoomIds = new Set(
		org.teams.flatMap((team) =>
			team.conversation_id ? [team.conversation_id] : [],
		),
	);

	const nodes = flattenForest(orgForest(org));
	const teammates = teammateTeams(nodes, me);
	const teammateChats = new Map<string, IConversation>();
	for (const conversation of conversations) {
		const partner = personPartner(conversation, me);
		if (partner && teammates.has(partner) && !teammateChats.has(partner))
			teammateChats.set(partner, conversation);
	}
	const teamChatIds = new Set(
		[...teammateChats.values()].map((item) => item.id),
	);
	const peopleRows = teammateRows(input, teammates, teammateChats);

	const mine = myChats(org, me, byId);
	const chatIds = new Set([...mine.active.values()].map((item) => item.id));
	const groupedThreadIds = new Set(
		(org.threads ?? []).flatMap((thread) =>
			teamById(org, thread.team_id) ? [thread.conversation_id] : [],
		),
	);
	const loose = conversations
		.filter(
			(item) =>
				!directIds.has(item.id) &&
				!teamRoomIds.has(item.id) &&
				!teamChatIds.has(item.id) &&
				!chatIds.has(item.id) &&
				!groupedThreadIds.has(item.id),
		)
		.sort(byActivity);
	const rowsOf = (items: readonly IConversation[]) =>
		items.map((item) => conversationRow(input, item));
	const pinned: ChatRow[] = [
		...rowsOf(
			[...loose, ...teammateChats.values()].filter((item) => item.pinned_at),
		),
		...pinnedBotRows(input, directChats),
	].sort((a, b) => pinnedAt(b) - pinnedAt(a));
	const unpinned = loose.filter((item) => !item.pinned_at);
	const groupOf = (kind: ChatGroupKind, rows: ChatRow[]): ChatGroup => ({
		key: kind,
		kind,
		team: null,
		depth: 0,
		rows,
		unread: totalUnread(rows),
	});

	const teams = nodes.map((node) =>
		teamGroup(
			input,
			node,
			crewById,
			directChats,
			byId,
			peopleRows.get(node.team.id) ?? [],
			mine,
		),
	);
	const placed = new Set(
		teams.flatMap((group) => group.rows.map((row) => row.key)),
	);
	const strayChats = [...mine.active].flatMap(([botId, conversation]) => {
		const bot = orgBot(org, botId);
		return bot && !placed.has(`conversation:${conversation.id}`)
			? [botChatRow(input, bot, conversation)]
			: [];
	});
	const soloRows = crew.flatMap((member) => {
		const id = crewId(member);
		const chat = directChats.get(id);
		if (activeTeamOf(org, id) || chat?.pinned_at) return [];
		return [botRow(input, member, chat ?? null, false)];
	});

	const shown = [
		groupOf("pinned", pinned),
		...teams,
		groupOf("bots", soloRows),
		groupOf("colleagues", strayChats.sort(rowsByActivity)),
		groupOf("rooms", rowsOf(unpinned.filter((item) => item.kind === "group"))),
		groupOf("people", rowsOf(unpinned.filter((item) => item.kind !== "group"))),
		groupOf("watching", watchingRows(input, watching)),
	].flatMap((group) => {
		const kept = filtered(input, group);
		return kept ? [kept] : [];
	});
	return input.filter === "all" && !input.query ? shown : withoutRepeats(shown);
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
