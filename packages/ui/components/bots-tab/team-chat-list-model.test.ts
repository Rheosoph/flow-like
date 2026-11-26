import { describe, expect, test } from "bun:test";
import type {
	IOrgBot,
	IOrgView,
	ITeam,
} from "../../state/backend-state/bot-team-state";
import type { IConversation } from "../../state/backend-state/inbox-state";
import type { CrewMember } from "./crew-grid";
import {
	type ChatListInput,
	buildChatGroups,
	teamRooms,
} from "./team-chat-list-model";

const ME = "mara";

function conversation(
	id: string,
	over: Partial<IConversation> = {},
): IConversation {
	return {
		id,
		kind: "group",
		title: id,
		encryption: "none",
		created_by: ME,
		created_at: "2026-01-01T00:00:00Z",
		last_activity_at: "2026-01-01T00:00:00Z",
		members: [],
		unread_count: 0,
		last_entry: null,
		...over,
	} as IConversation;
}

function direct(
	id: string,
	partner: string,
	over: Partial<IConversation> = {},
) {
	return conversation(id, {
		kind: "direct",
		members: [
			{ principal_id: ME, kind: "user" },
			{
				principal_id: partner,
				kind: partner.startsWith("bot") ? "bot" : "user",
			},
		] as IConversation["members"],
		...over,
	});
}

function member(id: string, name = id): CrewMember {
	return {
		key: id,
		selection: { type: "bot", botId: id },
		bot: { id } as CrewMember["bot"],
		system: null,
		name,
		description: `${name} job`,
		encrypted: false,
		local: false,
	};
}

function orgBot(id: string, owner: string): IOrgBot {
	return {
		id,
		owner_id: owner,
		appearance: { kind: "robot" },
		location: "server",
		description: `${id} job`,
	};
}

function team(id: string, over: Partial<ITeam> = {}): ITeam {
	return {
		id,
		name: id,
		direct_asks: false,
		people: [],
		bots: [],
		created_at: "",
		updated_at: "",
		...over,
	};
}

const org: IOrgView = {
	invitations: [],
	bots: [
		orgBot("botMine", ME),
		orgBot("botLead", "jonas"),
		orgBot("botPend", "anna"),
	],
	teams: [
		team("Kestrel", { conversation_id: "kroom" }),
		team("IT Support", {
			parent_id: "Kestrel",
			conversation_id: "itroom",
			lead_bot_id: "botLead",
			bots: [
				{
					bot_id: "botPend",
					owner_id: "anna",
					status: "pending",
					direct_agreed: false,
				},
				{
					bot_id: "botMine",
					owner_id: ME,
					status: "active",
					direct_agreed: false,
				},
				{
					bot_id: "botLead",
					owner_id: "jonas",
					status: "active",
					direct_agreed: false,
				},
			],
		}),
	],
};

const crew = [member("botMine", "Mine"), member("botFree", "Free")];

function input(over: Partial<ChatListInput> = {}): ChatListInput {
	return {
		org,
		me: ME,
		conversations: [
			conversation("kroom", { unread_count: 2 }),
			conversation("itroom"),
			direct("d1", "botMine", { unread_count: 1 }),
			conversation("r1", { last_activity_at: "2026-02-01T00:00:00Z" }),
			conversation("r2", { pinned_at: "2026-03-01T00:00:00Z" }),
			direct("p1", "jonas"),
		],
		crew,
		filter: "all",
		query: "",
		title: (c) => c.title ?? c.id,
		snippet: (c) => `last of ${c.id}`,
		orgBotName: (b) => b.id,
		ownerName: (id) => id,
		...over,
	};
}

const keys = (groups: ReturnType<typeof buildChatGroups>) =>
	groups.map((group) => group.key);

describe("buildChatGroups", () => {
	test("orders pinned, teams in org order, your bots, rooms, people", () => {
		expect(keys(buildChatGroups(input()))).toEqual([
			"pinned",
			"team:Kestrel",
			"team:IT Support",
			"bots",
			"rooms",
			"people",
		]);
	});

	test("a team opens with its room, then the lead, then members, pending last", () => {
		const group = buildChatGroups(input()).find(
			(g) => g.key === "team:IT Support",
		);
		expect(group?.depth).toBe(1);
		expect(group?.rows.map((row) => row.key)).toEqual([
			"conversation:itroom",
			"colleague:IT Support:botLead",
			"bot:botMine",
			"colleague:IT Support:botPend",
		]);
		const room = group?.rows[0];
		expect(room?.kind === "conversation" && room.tag).toBe("#ITSupport");
		const pending = group?.rows[3];
		expect(pending?.kind === "colleague" && pending.pending).toBe(true);
	});

	test("an own bot in a team leaves Your bots; its direct chat feeds its row", () => {
		const groups = buildChatGroups(input());
		const bots = groups.find((g) => g.key === "bots");
		expect(bots?.rows.map((row) => row.key)).toEqual(["bot:botFree"]);
		const mine = groups
			.find((g) => g.key === "team:IT Support")
			?.rows.find((row) => row.key === "bot:botMine");
		expect(mine?.unread).toBe(1);
		expect(mine?.snippet).toBe("last of d1");
	});

	test("team rooms and bot chats stay out of Rooms and People", () => {
		const groups = buildChatGroups(input());
		expect(
			groups.find((g) => g.key === "rooms")?.rows.map((r) => r.key),
		).toEqual(["conversation:r1"]);
		expect(
			groups.find((g) => g.key === "people")?.rows.map((r) => r.key),
		).toEqual(["conversation:p1"]);
		expect(groups[0].rows.map((r) => r.key)).toEqual(["conversation:r2"]);
	});

	test("unread filter keeps only rows with unread", () => {
		const groups = buildChatGroups(input({ filter: "unread" }));
		expect(groups.flatMap((g) => g.rows.map((r) => r.key))).toEqual([
			"conversation:kroom",
			"bot:botMine",
		]);
	});

	test("pinned filter keeps only pinned rows", () => {
		const groups = buildChatGroups(input({ filter: "pinned" }));
		expect(groups.flatMap((g) => g.rows.map((r) => r.key))).toEqual([
			"conversation:r2",
		]);
	});

	test("search matches name, snippet and team name", () => {
		const byName = buildChatGroups(input({ query: "free" }));
		expect(byName.flatMap((g) => g.rows.map((r) => r.key))).toEqual([
			"bot:botFree",
		]);
		const byTeam = buildChatGroups(input({ query: "it support" }));
		expect(keys(byTeam)).toEqual(["team:IT Support"]);
		expect(byTeam[0].rows).toHaveLength(4);
		const bySnippet = buildChatGroups(input({ query: "last of r1" }));
		expect(bySnippet.flatMap((g) => g.rows.map((r) => r.key))).toEqual([
			"conversation:r1",
		]);
	});

	test("without teams only the plain groups remain", () => {
		const groups = buildChatGroups(
			input({ org: { teams: [], bots: [], invitations: [] } }),
		);
		expect(keys(groups)).toEqual(["pinned", "bots", "rooms", "people"]);
	});
});

describe("teamRooms", () => {
	test("lists the rooms present in the conversations, in org order", () => {
		expect(teamRooms(org, input().conversations).map((r) => r.team.id)).toEqual(
			["Kestrel", "IT Support"],
		);
	});
});
