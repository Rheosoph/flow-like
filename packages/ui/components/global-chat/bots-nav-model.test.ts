import { describe, expect, test } from "vitest";
import { IRole } from "../../lib/schema/llm/history";
import type {
	IConversation,
	IConversationMember,
	IInboxEntry,
} from "../../state/backend-state/inbox-state";
import { botConversationId } from "../../state/global-chat/global-chat-bots";
import {
	GLOBAL_CHAT_APP_ID,
	type IMessage,
} from "../../state/global-chat/global-chat-db";
import type {
	GlobalChatRun,
	GlobalToolPrompt,
} from "../../state/global-chat/global-chat-store";
import {
	latestBotActivity,
	latestInboxBotActivity,
	latestLiveBotActivity,
	navBotAgentState,
} from "./bots-nav-model";

const at = (minutes: number) => Date.UTC(2026, 10, 5, 12, minutes);
const iso = (minutes: number) => new Date(at(minutes)).toISOString();

function member(
	principal_id: string,
	kind: IConversationMember["kind"] = "user",
): IConversationMember {
	return { principal_id, kind, added_by: null, joined_at: iso(0) };
}

function entry(sender_id: string, minutes: number): IInboxEntry {
	return {
		id: `entry-${sender_id}-${minutes}`,
		message_id: `message-${sender_id}-${minutes}`,
		conversation_id: "conversation",
		sender_id,
		kind: "message",
		envelope: { v: 1, enc: "none", body: { text: "Hello" } },
		created_at: iso(minutes),
		cursor: `${minutes}:entry`,
	};
}

function conversation(
	botId: string,
	last_entry: IInboxEntry | null,
	overrides: Partial<IConversation> = {},
): IConversation {
	return {
		id: `conversation-${botId}`,
		kind: "direct",
		title: null,
		encryption: "plain",
		created_by: "me",
		created_at: iso(0),
		last_activity_at: iso(0),
		members: [member("me"), member(botId, "bot")],
		unread_count: 0,
		last_entry,
		...overrides,
	};
}

function message(
	botId: string | null,
	minutes: number,
	overrides: Partial<IMessage> = {},
): IMessage {
	return {
		id: `message-${botId}-${minutes}`,
		appId: GLOBAL_CHAT_APP_ID,
		sessionId: botId === null ? "flux-session" : botConversationId(botId),
		inner: { role: IRole.User, content: "Hello" },
		files: [],
		timestamp: at(minutes),
		...overrides,
	};
}

function run(botId: string | null, minutes: number): GlobalChatRun {
	return {
		runId: `run-${botId}`,
		conversationId: botId === null ? "flux-session" : botConversationId(botId),
		startedAt: at(minutes),
		status: "streaming",
		label: "Hello",
		message: null,
		sourceAttachments: [],
		selection: {
			runId: `run-${botId}`,
			provider: "bits",
			selectedModelId: "model",
			reasoningEffort: "",
		},
		subPlanSteps: [],
		subAttachments: [],
		subUsageStats: [],
		subWidgets: [],
		pendingAppRefs: [],
		debugReport: null,
		steers: [],
	};
}

function prompt(overrides: Partial<GlobalToolPrompt> = {}): GlobalToolPrompt {
	return {
		id: "prompt",
		kind: "approval",
		toolName: "tool",
		title: "Allow this action?",
		respond: () => {},
		...overrides,
	};
}

describe("navigation bot recency", () => {
	test("follows both outgoing messages and incoming replies, regardless of pins and reads", () => {
		const old = conversation("bot_old", entry("bot_old", 1), {
			pinned_at: iso(10),
			last_activity_at: iso(10),
			unread_count: 4,
		});
		const outgoing = conversation("bot_new", entry("me", 2));
		expect(latestInboxBotActivity([old, outgoing])).toEqual({
			botId: "bot_new",
			at: at(2),
		});
		const reply = conversation("bot_old", entry("bot_old", 3));
		expect(latestInboxBotActivity([outgoing, reply])).toEqual({
			botId: "bot_old",
			at: at(3),
		});
	});

	test("ignores empty chats, person chats, metadata entries and invalid dates", () => {
		const valid = conversation("bot_valid", entry("me", 1));
		const person = conversation("person", entry("person", 9), {
			members: [member("me"), member("person")],
		});
		const metadata = conversation("bot_metadata", {
			...entry("bot_metadata", 10),
			kind: "reaction",
		});
		const invalid = conversation("bot_invalid", {
			...entry("bot_invalid", 11),
			created_at: "invalid",
		});
		expect(
			latestInboxBotActivity([
				conversation("bot_empty", null),
				person,
				metadata,
				invalid,
				valid,
			]),
		).toEqual({ botId: "bot_valid", at: at(1) });
	});

	test("uses the bot who wrote in a group and ignores ambiguous human recipients", () => {
		const group = conversation("bot_a", entry("bot_b", 2), {
			kind: "group",
			members: [member("me"), member("bot_a", "bot"), member("bot_b", "bot")],
		});
		expect(latestInboxBotActivity([group])).toEqual({
			botId: "bot_b",
			at: at(2),
		});
		expect(
			latestInboxBotActivity([{ ...group, last_entry: entry("me", 3) }]),
		).toBeNull();
	});

	test("chooses a new local send before inbox refresh, including Flux", () => {
		const inbox = latestInboxBotActivity([
			conversation("bot_inbox", entry("bot_inbox", 1)),
		]);
		const live = latestLiveBotActivity({
			messages: [message(null, 2)],
			runs: {},
		});
		expect(latestBotActivity(inbox, undefined, live)).toEqual({
			botId: null,
			at: at(2),
		});
	});

	test("an older background reply finishing after a newer send becomes most recent", () => {
		const background = run("bot_background", 1);
		background.message = message("bot_background", 1, {
			inner: { role: IRole.Assistant, content: "Finished" },
			run_context: { schema: "1", ended_at_ms: at(3) },
		});
		expect(
			latestLiveBotActivity({
				messages: [message("bot_foreground", 2)],
				runs: { [background.runId]: background },
			}),
		).toEqual({ botId: "bot_background", at: at(3) });
	});

	test("opening an empty chat does not replace the last exchange", () => {
		const recent = { botId: "bot_last", at: at(1) };
		const empty = latestLiveBotActivity({ messages: [], runs: {} });
		expect(empty).toBeNull();
		expect(latestBotActivity(recent, empty)).toBe(recent);
	});
});

describe("navigation bot state", () => {
	test("only the matching bot works, including a background Flux run", () => {
		const state = {
			runs: { flux: run(null, 1) },
			toolPrompt: null,
			activeConversationId: botConversationId("bot_other"),
		};
		expect(navBotAgentState(state, null)).toBe("working");
		expect(navBotAgentState(state, "bot_other")).toBe("idle");
	});

	test("an approval follows its run instead of the open conversation", () => {
		const working = run("bot_working", 1);
		const state = {
			runs: { [working.runId]: working },
			toolPrompt: prompt({ runId: working.runId }),
			activeConversationId: botConversationId("bot_other"),
		};
		expect(navBotAgentState(state, "bot_working")).toBe("needs-you");
		expect(navBotAgentState(state, "bot_other")).toBe("idle");
	});

	test("an approval without a run uses its conversation, then the open chat", () => {
		const state = {
			runs: {},
			toolPrompt: prompt({ conversationId: botConversationId("bot_asking") }),
			activeConversationId: botConversationId("bot_other"),
		};
		expect(navBotAgentState(state, "bot_asking")).toBe("needs-you");
		expect(navBotAgentState(state, "bot_other")).toBe("idle");
		state.toolPrompt = prompt();
		expect(navBotAgentState(state, "bot_other")).toBe("needs-you");
	});
});
