import { describe, expect, test } from "bun:test";
import type {
	IBotChat,
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

	test("a pinned bot shows in Pinned by pin time, stays in its team, leaves Your bots", () => {
		const groups = buildChatGroups(
			input({
				conversations: [
					conversation("itroom"),
					direct("d1", "botMine", { pinned_at: "2026-03-02T00:00:00Z" }),
					direct("d2", "botFree", { pinned_at: "2026-02-01T00:00:00Z" }),
					conversation("r2", { pinned_at: "2026-03-01T00:00:00Z" }),
				],
			}),
		);
		expect(groups[0].rows.map((r) => r.key)).toEqual([
			"bot:botMine",
			"conversation:r2",
			"bot:botFree",
		]);
		const it = groups.find((g) => g.key === "team:IT Support");
		expect(it?.rows.map((r) => r.key)).toContain("bot:botMine");
		expect(keys(groups)).not.toContain("bots");
	});

	test("filtering or searching shows a pinned team bot once, in Pinned", () => {
		const conversations = [
			direct("d1", "botMine", {
				pinned_at: "2026-03-02T00:00:00Z",
				unread_count: 1,
			}),
		];
		for (const over of [
			{ filter: "pinned" as const },
			{ filter: "unread" as const },
			{ query: "mine" },
		]) {
			const groups = buildChatGroups(input({ conversations, ...over }));
			expect(groups.map((g) => [g.key, g.rows.map((r) => r.key)])).toEqual([
				["pinned", ["bot:botMine"]],
			]);
		}
	});

	test("without teams only the plain groups remain", () => {
		const groups = buildChatGroups(
			input({ org: { teams: [], bots: [], invitations: [] } }),
		);
		expect(keys(groups)).toEqual(["pinned", "bots", "rooms", "people"]);
	});
});

describe("teammates", () => {
	const person = (user_id: string, status: "active" | "invited" = "active") =>
		({ user_id, role: "member", status }) as ITeam["people"][number];
	const withPeople = (over: Partial<ChatListInput> = {}) => {
		const [kestrel, it] = org.teams;
		return input({
			org: {
				...org,
				teams: [
					{
						...kestrel,
						people: [
							person(ME),
							person("jonas"),
							person("anna"),
							person("ida", "invited"),
						],
					},
					{ ...it, people: [person(ME), person("jonas")] },
					team("Elsewhere", { people: [person("lea")] }),
				],
			},
			...over,
		});
	};
	const rowsOf = (groups: ReturnType<typeof buildChatGroups>, key: string) =>
		groups.find((g) => g.key === key)?.rows.map((r) => r.key);

	test("a teammate's direct chat moves from People into the most specific shared team", () => {
		const groups = buildChatGroups(withPeople());
		expect(rowsOf(groups, "team:IT Support")?.at(-1)).toBe("conversation:p1");
		expect(rowsOf(groups, "team:Kestrel")).not.toContain("conversation:p1");
		expect(keys(groups)).not.toContain("people");
	});

	test("a teammate without a chat gets a row to start one; invited people and strangers don't", () => {
		const groups = buildChatGroups(withPeople());
		expect(rowsOf(groups, "team:Kestrel")).toEqual([
			"conversation:kroom",
			"person:Kestrel:anna",
		]);
		const people = groups
			.flatMap((g) => g.rows.map((r) => r.key))
			.filter((key) => key.startsWith("person:"));
		expect(people).toEqual(["person:Kestrel:anna"]);
	});

	test("a pinned chat with a teammate shows in Pinned and stays in their team", () => {
		const conversations = [
			direct("p2", "anna", { pinned_at: "2026-03-01T00:00:00Z" }),
		];
		const groups = buildChatGroups(withPeople({ conversations }));
		expect(rowsOf(groups, "pinned")).toEqual(["conversation:p2"]);
		expect(rowsOf(groups, "team:Kestrel")).toEqual(["conversation:p2"]);
		const pinnedOnly = buildChatGroups(
			withPeople({ conversations, filter: "pinned" }),
		);
		expect(keys(pinnedOnly)).toEqual(["pinned"]);
	});

	test("search finds a teammate by name", () => {
		const groups = buildChatGroups(withPeople({ query: "anna" }));
		expect(rowsOf(groups, "team:Kestrel")).toEqual(["person:Kestrel:anna"]);
	});
});

describe("your requests", () => {
	const withThreads = (
		threads: NonNullable<IOrgView["threads"]>,
		over: Partial<ChatListInput> = {},
	) =>
		input({
			org: { ...org, threads },
			requestsLabel: "Your requests",
			conversations: [
				conversation("itroom"),
				conversation("thread1", { title: "thread" }),
				conversation("r1"),
			],
			...over,
		});

	test("the thread sits right after the team room, labelled", () => {
		const groups = buildChatGroups(
			withThreads([{ team_id: "IT Support", conversation_id: "thread1" }]),
		);
		const rows = groups.find((g) => g.key === "team:IT Support")?.rows;
		expect(rows?.slice(0, 2).map((row) => row.key)).toEqual([
			"conversation:itroom",
			"conversation:thread1",
		]);
		expect(rows?.[1]).toMatchObject({ name: "Your requests", thread: true });
	});

	test("it does not appear again under Rooms or Pinned", () => {
		const groups = buildChatGroups(
			withThreads([{ team_id: "IT Support", conversation_id: "thread1" }], {
				conversations: [
					conversation("itroom"),
					conversation("thread1", { pinned_at: "2026-03-01T00:00:00Z" }),
					conversation("r1"),
				],
			}),
		);
		expect(keys(groups)).not.toContain("pinned");
		expect(
			groups.find((g) => g.key === "rooms")?.rows.map((r) => r.key),
		).toEqual(["conversation:r1"]);
	});

	test("a thread of a team the reader's org view lacks stays under Rooms", () => {
		const groups = buildChatGroups(
			withThreads([{ team_id: "gone", conversation_id: "thread1" }]),
		);
		expect(
			groups.find((g) => g.key === "rooms")?.rows.map((r) => r.key),
		).toEqual(["conversation:thread1", "conversation:r1"]);
	});
});

describe("bot chats and watching", () => {
	const chat = (over: Partial<IBotChat> = {}): IBotChat => ({
		bot_id: "botLead",
		user_id: ME,
		owner_id: "jonas",
		status: "active",
		conversation_id: "chat1",
		requested_at: "2026-01-01T00:00:00Z",
		...over,
	});
	const withChats = (
		chats: IBotChat[],
		conversations: IConversation[],
		over: Partial<ChatListInput> = {},
	) => input({ org: { ...org, chats }, conversations, ...over });
	const rowKeys = (groups: ReturnType<typeof buildChatGroups>, key: string) =>
		groups.find((group) => group.key === key)?.rows.map((row) => row.key);

	test("an approved chat sits in the bot's team and not in Rooms", () => {
		const groups = buildChatGroups(
			withChats([chat()], [conversation("chat1", { unread_count: 3 })]),
		);
		expect(rowKeys(groups, "team:IT Support")).toContain("conversation:chat1");
		expect(rowKeys(groups, "team:IT Support")).not.toContain(
			"colleague:IT Support:botLead",
		);
		expect(groups.find((group) => group.key === "rooms")).toBeUndefined();
		const row = groups
			.flatMap((group) => group.rows)
			.find((item) => item.key === "conversation:chat1");
		expect(row?.name).toBe("botLead");
		expect(row?.unread).toBe(3);
	});

	test("an approved chat of a team the list doesn't show goes to Colleagues' bots", () => {
		const groups = buildChatGroups(
			withChats([chat({ bot_id: "botStray" })], [conversation("chat1")], {
				org: {
					...org,
					bots: [...org.bots, orgBot("botStray", "jonas")],
					chats: [chat({ bot_id: "botStray" })],
				},
			}),
		);
		expect(rowKeys(groups, "colleagues")).toEqual(["conversation:chat1"]);
	});

	test("a pending request waits as a muted row, a declined one shows nothing", () => {
		const pending = buildChatGroups(
			withChats(
				[chat({ status: "pending", conversation_id: null })],
				[conversation("itroom")],
			),
		);
		const waiting = pending
			.flatMap((group) => group.rows)
			.find((row) => row.key === "colleague:IT Support:botLead");
		expect(waiting?.kind === "colleague" && waiting.pending).toBe(true);
		const declined = buildChatGroups(
			withChats(
				[chat({ status: "declined", conversation_id: null })],
				[conversation("itroom")],
			),
		);
		expect(
			declined
				.flatMap((group) => group.rows)
				.filter((row) => row.key.startsWith("conversation:")),
		).toHaveLength(1);
	});

	test("other people's chats with the reader's bot are not the reader's chats", () => {
		const groups = buildChatGroups(
			withChats(
				[chat({ bot_id: "botMine", user_id: "tom", owner_id: ME })],
				[conversation("chat1", { watching: true })],
			),
		);
		expect(rowKeys(groups, "team:IT Support")).not.toContain(
			"conversation:chat1",
		);
	});

	test("watching conversations list last, quiet, named after whose they are", () => {
		const groups = buildChatGroups(
			withChats(
				[chat({ bot_id: "botMine", user_id: "tom", owner_id: ME })],
				[
					conversation("r1", { unread_count: 1 }),
					conversation("chat1", { watching: true, unread_count: 5 }),
					conversation("thr", {
						watching: true,
						unread_count: 2,
						title: "Thread of tom",
					}),
				],
			),
		);
		expect(keys(groups).at(-1)).toBe("watching");
		const watching = groups.at(-1);
		expect(watching?.rows.map((row) => row.name).sort()).toEqual([
			"Thread of tom",
			"tom \u00b7 botMine",
		]);
		expect(watching?.rows.every((row) => row.unread === 0)).toBe(true);
		expect(watching?.unread).toBe(0);
		expect(groups.find((group) => group.key === "rooms")?.unread).toBe(1);
	});

	test("watching conversations stay out of unread and pinned filters, and match search", () => {
		const conversations = [
			conversation("chat1", {
				watching: true,
				unread_count: 5,
				pinned_at: "2026-03-01T00:00:00Z",
			}),
		];
		expect(buildChatGroups(input({ conversations, filter: "unread" }))).toEqual(
			[],
		);
		expect(buildChatGroups(input({ conversations, filter: "pinned" }))).toEqual(
			[],
		);
		expect(
			keys(buildChatGroups(input({ conversations, query: "chat1" }))),
		).toEqual(["watching"]);
	});
});

describe("teamRooms", () => {
	test("lists the rooms present in the conversations, in org order", () => {
		expect(teamRooms(org, input().conversations).map((r) => r.team.id)).toEqual(
			["Kestrel", "IT Support"],
		);
	});
});
