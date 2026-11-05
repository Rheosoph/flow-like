// @vitest-environment happy-dom
import "fake-indexeddb/auto";
import { beforeEach, describe, expect, test } from "vitest";
import { IRole } from "../../lib/schema/llm/history";
import type { IInboxEntry } from "../backend-state/inbox-state";
import { botConversationId } from "./global-chat-bots";
import type { IMessage } from "./global-chat-db";
import {
	beginGlobalChatTurnSelection,
	useGlobalChatStore,
} from "./global-chat-store";
import { followPushedEntries } from "./use-follow-bot-chat";

const BOT = "bot_ada";
const SESSION = botConversationId(BOT);

function message(id: string, role: IRole, timestamp: number): IMessage {
	return {
		id,
		appId: "global",
		sessionId: SESSION,
		inner: { role, content: id },
		files: [],
		tools: [],
		actions: [],
		timestamp,
	};
}

function storedReply(replyTo: string): IInboxEntry {
	return {
		id: "e2",
		message_id: `reply_${replyTo}`,
		conversation_id: "conv_1",
		sender_id: BOT,
		kind: "message",
		envelope: {
			v: 1,
			enc: "none",
			body: { text: "Done", reply_to: replyTo, hops: 1 },
		},
		created_at: new Date(Date.UTC(2026, 10, 4, 17, 44)).toISOString(),
		cursor: "2000:e2",
		seq: 2,
	};
}

const shownIds = () =>
	useGlobalChatStore.getState().messages.map((shown) => shown.id);

describe("following a bot chat", () => {
	beforeEach(() => {
		useGlobalChatStore.setState({
			activeConversationId: SESSION,
			messages: [],
			runs: {},
			queue: [],
			isStreaming: false,
			streamingMessages: [],
		});
		useGlobalChatStore.getState().appendMessage(message("m1", IRole.User, 1));
	});

	test("the reply the host stores while the answer still streams is not shown twice", () => {
		const answer = message("run_1", IRole.Assistant, 2);
		useGlobalChatStore.getState().startRun({
			runId: answer.id,
			conversationId: SESSION,
			selection: beginGlobalChatTurnSelection(answer.id),
			label: "turn",
			message: answer,
			sourceAttachments: [],
		});

		followPushedEntries(BOT, SESSION, [storedReply("m1")]);
		useGlobalChatStore.getState().commitMessage(answer);
		useGlobalChatStore.getState().endRun(answer.id);

		expect(shownIds()).toEqual(["m1", "run_1"]);
	});

	test("a reply to a message no run on screen answers comes through", () => {
		followPushedEntries(BOT, SESSION, [storedReply("m1")]);

		expect(shownIds()).toEqual(["m1", "reply_m1"]);
	});
});
