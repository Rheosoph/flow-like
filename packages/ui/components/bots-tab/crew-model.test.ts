import { describe, expect, test } from "bun:test";
import type {
	IConversation,
	IConversationMember,
	IInboxEntry,
} from "../../state/backend-state/inbox-state";
import {
	activityTimeKind,
	awaitingAnswer,
	conversationComposition,
	crewSummaryLine,
	directPartnerId,
	findDirectConversation,
	isSelfPrincipal,
	matchesSearch,
	otherMemberIds,
	personUnreadTotal,
	selectNeedsYou,
	splitConversations,
} from "./crew-model";
import {
	DEFAULT_DOCK_SELECTION,
	dockSelectionForBot,
	dockSelectionKey,
	dockSelectionQuery,
	isMoveRequested,
	parseDockSelection,
	resolveDockSelection,
	withoutMoveRequest,
} from "./dock-selection";

function member(
	principal_id: string,
	kind: IConversationMember["kind"] = "user",
): IConversationMember {
	return { principal_id, kind, added_by: null, joined_at: "" };
}

function lastEntry(sender_id: string): IInboxEntry {
	return {
		id: `e_${sender_id}`,
		message_id: `m_${sender_id}`,
		conversation_id: "c",
		sender_id,
		kind: "message",
		envelope: { v: 1, enc: "none", body: { text: "hi" } },
		created_at: "2026-11-01T09:00:00Z",
		cursor: "1:e",
	};
}

function approvalEntry(sender_id: string): IInboxEntry {
	return {
		...lastEntry(sender_id),
		envelope: {
			v: 1,
			enc: "none",
			body: {
				text: "Run git",
				approval: {
					id: "ap-1",
					turn_id: "t1",
					tool: "shell_run",
					summary: "Run git",
					scope: "command:git",
					args_hash: "h",
				},
			},
		},
	};
}

function conversation(
	id: string,
	overrides: Partial<IConversation> = {},
): IConversation {
	return {
		id,
		kind: "group",
		title: id,
		encryption: "plain",
		created_by: "me",
		created_at: "2026-11-01T08:00:00Z",
		last_activity_at: "2026-11-01T09:00:00Z",
		members: [member("me"), member("anna"), member("bot_scout", "bot")],
		unread_count: 0,
		last_entry: null,
		...overrides,
	};
}

describe("selectNeedsYou", () => {
	const myBotIds = new Set(["bot_mine"]);

	test("ordinary unread bot replies do not need the reader's input", () => {
		const fromBot = conversation("bot", {
			unread_count: 2,
			last_entry: lastEntry("bot_scout"),
		});
		const fromPerson = conversation("person", {
			unread_count: 1,
			last_entry: lastEntry("anna"),
		});
		const read = conversation("read", { last_entry: lastEntry("bot_scout") });
		const leftBot = conversation("left", {
			unread_count: 1,
			members: [member("me")],
			last_entry: lastEntry("bot_mine"),
		});
		expect(
			selectNeedsYou([fromBot, fromPerson, read, leftBot], {
				me: "me",
				myBotIds,
			}).map(({ id }) => id),
		).toEqual([]);
	});

	test("an approval card stays until an answer follows, read or not", () => {
		const card = approvalEntry("bot_scout");
		const waiting = conversation("waiting", { last_entry: card });
		const unread = conversation("unread", {
			last_entry: card,
			unread_count: 1,
		});
		const answered = conversation("answered", {
			last_entry: lastEntry("me"),
		});
		const finished = conversation("finished", {
			last_entry: lastEntry("bot_scout"),
			unread_count: 1,
		});
		expect(
			selectNeedsYou([waiting, unread, answered, finished], {
				me: "me",
				myBotIds,
			}).map(({ id }) => id),
		).toEqual(["waiting", "unread"]);
	});

	test("skips the conversation already open and my own messages", () => {
		const open = conversation("open", {
			unread_count: 1,
			last_entry: approvalEntry("bot_scout"),
		});
		const mine = conversation("mine", {
			unread_count: 1,
			members: [member("me", "bot")],
			last_entry: approvalEntry("me"),
		});
		expect(
			selectNeedsYou([open, mine], {
				me: "me",
				myBotIds,
				openConversationId: "open",
			}),
		).toEqual([]);
	});

	test("hides the approval in the bot chat on screen, keeping other chats' approvals", () => {
		const direct = conversation("direct", {
			kind: "direct",
			members: [member("me"), member("bot_scout", "bot")],
			last_entry: approvalEntry("bot_scout"),
		});
		const room = conversation("room", {
			last_entry: approvalEntry("bot_scout"),
		});
		const other = conversation("other", {
			kind: "direct",
			members: [member("local"), member("bot_mine", "bot")],
			last_entry: approvalEntry("bot_mine"),
		});
		const conversations = [direct, room, other];
		expect(
			selectNeedsYou(conversations, {
				me: "me",
				myBotIds,
				openBotId: "bot_scout",
			}).map(({ id }) => id),
		).toEqual(["room", "other"]);
		expect(
			selectNeedsYou(conversations, {
				me: "me",
				myBotIds,
				openBotId: "bot_mine",
			}).map(({ id }) => id),
		).toEqual(["direct", "room"]);
		expect(
			selectNeedsYou(conversations, { me: "me", myBotIds }).map(({ id }) => id),
		).toEqual(["direct", "room", "other"]);
	});
});

describe("isSelfPrincipal", () => {
	test("is the account's sub and the signed-out `local`, nobody else", () => {
		expect(isSelfPrincipal("me", "me")).toBe(true);
		expect(isSelfPrincipal("local", "me")).toBe(true);
		expect(isSelfPrincipal("local", "local")).toBe(true);
		expect(isSelfPrincipal("local", null)).toBe(true);
		expect(isSelfPrincipal("anna", "me")).toBe(false);
		expect(isSelfPrincipal("bot_scout", "local")).toBe(false);
		expect(isSelfPrincipal(undefined, "me")).toBe(false);
		expect(isSelfPrincipal("", "")).toBe(false);
	});

	test("a chat on this device treats `local` as the signed-in reader", () => {
		const local = conversation("local", {
			kind: "direct",
			local: true,
			members: [member("local"), member("bot_local", "bot")],
			unread_count: 1,
			last_entry: lastEntry("local"),
		});
		expect(otherMemberIds(local, "me")).toEqual(["bot_local"]);
		expect(directPartnerId(local, "me")).toBe("bot_local");
		expect(findDirectConversation([local], "bot_local", "me")?.id).toBe(
			"local",
		);
		expect(
			selectNeedsYou([local], { me: "me", myBotIds: new Set(["bot_local"]) }),
		).toEqual([]);
	});
});

describe("conversation helpers", () => {
	test("counts people and bots including the reader", () => {
		expect(conversationComposition(conversation("c"))).toEqual({
			people: 2,
			bots: 1,
		});
	});

	test("finds the partner of a direct conversation", () => {
		const direct = conversation("d", {
			kind: "direct",
			members: [member("me"), member("bot_scout", "bot")],
		});
		expect(directPartnerId(direct, "me")).toBe("bot_scout");
		expect(directPartnerId(conversation("g"), "me")).toBeUndefined();
		expect(findDirectConversation([direct], "bot_scout", "me")?.id).toBe("d");
		expect(findDirectConversation([direct], "anna", "me")).toBeUndefined();
	});

	test("splits rooms from direct conversations", () => {
		const direct = conversation("d", { kind: "direct" });
		const { rooms, direct: directs } = splitConversations([
			conversation("g"),
			direct,
		]);
		expect(rooms.map(({ id }) => id)).toEqual(["g"]);
		expect(directs.map(({ id }) => id)).toEqual(["d"]);
	});
});

describe("matchesSearch", () => {
	test("needs every word somewhere, ignoring case and accents", () => {
		expect(matchesSearch("", ["anything"])).toBe(true);
		expect(matchesSearch("q4 anna", ["Q4 launch", "Anna Weber"])).toBe(true);
		expect(matchesSearch("zoë", ["Zoe"])).toBe(true);
		expect(matchesSearch("ZOE", ["Zoë"])).toBe(true);
		expect(matchesSearch("q4 lisbon", ["Q4 launch", null])).toBe(false);
	});
});

describe("crewSummaryLine", () => {
	const t = (key: string, options: Record<string, unknown>) => {
		const template =
			options.count === 1 && typeof options.defaultValue_one === "string"
				? options.defaultValue_one
				: String(options.defaultValue);
		return `${key}=${template.replace(/\{\{(\w+)\}\}/g, (_, name: string) =>
			String(options[name]),
		)}`;
	};

	test("says nothing needs you and counts bots", () => {
		expect(crewSummaryLine(t, { needs: 0, bots: 1 })).toBe(
			"crew.summary.line=crew.summary.nothingNeedsYou=Nothing needs you · crew.summary.bots=1 bot",
		);
	});

	test("pluralizes and leaves out an unknown bot count", () => {
		expect(crewSummaryLine(t, { needs: 1, bots: null })).toBe(
			"crew.summary.needsYou=1 thing needs you",
		);
		expect(crewSummaryLine(t, { needs: 3, bots: 4 })).toBe(
			"crew.summary.line=crew.summary.needsYou=3 things need you · crew.summary.bots=4 bots",
		);
	});
});

describe("activityTimeKind", () => {
	const now = new Date(2026, 10, 5, 15, 0);

	test("buckets by calendar day", () => {
		expect(activityTimeKind(new Date(2026, 10, 5, 0, 5).getTime(), now)).toBe(
			"today",
		);
		expect(activityTimeKind(new Date(2026, 10, 4, 23, 0).getTime(), now)).toBe(
			"yesterday",
		);
		expect(activityTimeKind(new Date(2026, 10, 1, 9, 0).getTime(), now)).toBe(
			"week",
		);
		expect(activityTimeKind(new Date(2026, 9, 20).getTime(), now)).toBe(
			"older",
		);
		expect(activityTimeKind(Number.NaN, now)).toBeNull();
	});
});

describe("dock selection", () => {
	const params = (query: string) => new URLSearchParams(query);

	test("defaults to the Assistant and reads bots and conversations", () => {
		expect(parseDockSelection(params(""))).toEqual(DEFAULT_DOCK_SELECTION);
		expect(parseDockSelection(params("voice=1"))).toEqual(
			DEFAULT_DOCK_SELECTION,
		);
		expect(parseDockSelection(params("bot=flux"))).toEqual({
			type: "system",
			system: "flux",
		});
		expect(parseDockSelection(params("bot=bot_1"))).toEqual({
			type: "bot",
			botId: "bot_1",
		});
		expect(parseDockSelection(params("bot=bot_1&conversation=c1"))).toEqual({
			type: "conversation",
			conversationId: "c1",
		});
	});

	test("writes the selection and keeps other parameters", () => {
		expect(
			dockSelectionQuery({ type: "bot", botId: "bot 1" }, "voice=1&bot=flux"),
		).toBe("voice=1&bot=bot+1");
		expect(
			dockSelectionQuery(
				{ type: "conversation", conversationId: "c1" },
				params("bot=bot_1"),
			),
		).toBe("conversation=c1");
		expect(
			dockSelectionQuery(DEFAULT_DOCK_SELECTION, "conversation=c1&voice=1"),
		).toBe("voice=1");
		expect(dockSelectionQuery({ type: "system", system: "flux" }, "")).toBe(
			"bot=flux",
		);
	});

	test("opens built-in bots in the global chat even by id", () => {
		const bots = [
			{ id: "bot_general", system: "general" as const },
			{ id: "bot_custom", system: null },
		];
		expect(
			resolveDockSelection(
				{ type: "bot", botId: "bot_general" },
				bots as never,
			),
		).toEqual({ type: "system", system: "general" });
		expect(
			resolveDockSelection({ type: "bot", botId: "bot_custom" }, bots as never),
		).toEqual({ type: "bot", botId: "bot_custom" });
		expect(dockSelectionForBot(bots[0], bots as never)).toEqual({
			type: "system",
			system: "general",
		});
		expect(dockSelectionKey(dockSelectionForBot(bots[1], bots as never))).toBe(
			"bot:bot_custom",
		);
	});

	test("`?move=local` asks for the move review and leaves without the rest", () => {
		expect(isMoveRequested(params("move=local&bot=bot_1"))).toBe(true);
		expect(isMoveRequested(params("move=other"))).toBe(false);
		expect(isMoveRequested(params(""))).toBe(false);
		expect(withoutMoveRequest("move=local&bot=bot_1&voice=1")).toBe(
			"bot=bot_1&voice=1",
		);
		expect(withoutMoveRequest(params("move=local"))).toBe("");
		expect(parseDockSelection(params("move=local&conversation=c1"))).toEqual({
			type: "conversation",
			conversationId: "c1",
		});
		expect(
			dockSelectionQuery({ type: "bot", botId: "bot_1" }, "move=local"),
		).toBe("move=local&bot=bot_1");
	});
});

describe("person chat badge and pending state", () => {
	const person = (unread: number, users: number, local = false) => ({
		members: Array.from({ length: users }, () => ({
			kind: "user" as const,
		})) as never,
		unread_count: unread,
		local,
	});

	test("counts unread of person chats only", () => {
		expect(
			personUnreadTotal([
				person(2, 2),
				person(3, 3),
				person(5, 1),
				person(7, 2, true),
			]),
		).toBe(5);
	});

	test("a direct chat with a pending recipient awaits an answer", () => {
		expect(
			awaitingAnswer({
				kind: "direct",
				pending: [{ status: "pending" } as never],
			}),
		).toBe(true);
		expect(awaitingAnswer({ kind: "direct", pending: [] })).toBe(false);
		expect(
			awaitingAnswer({
				kind: "group",
				pending: [{ status: "pending" } as never],
			}),
		).toBe(false);
	});
});
