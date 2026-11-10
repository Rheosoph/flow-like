// @vitest-environment happy-dom

import "fake-indexeddb/auto";
import { beforeEach, describe, expect, test } from "vitest";
import type { IMessage } from "../../components/interfaces/chat-default/chat-db";
import {
	type IChatMessageInteraction,
	createMessageReference,
} from "../../components/interfaces/chat-default/chat-message-interaction";
import { IRole } from "../../lib/schema/llm/history";
import type { IUserContextRequest } from "../backend-state/inbox-state";
import { globalChatDb } from "./global-chat-db";
import {
	globalChatModelInput,
	persistGlobalChatReaction,
} from "./global-chat-message-interaction";
import { useGlobalChatStore } from "./global-chat-store";

const assistant = (): IMessage => ({
	id: "old-answer",
	appId: "global",
	sessionId: "conversation",
	timestamp: 1,
	inner: { role: IRole.Assistant, content: "The second option costs less." },
	files: [],
	rating: 1,
});

const reaction = (
	operation: "add" | "remove" = "add",
): IChatMessageInteraction => ({
	type: "reaction",
	emoji: "👍",
	operation,
	target: createMessageReference(assistant()),
});

beforeEach(async () => {
	await globalChatDb.messages.clear();
	useGlobalChatStore.setState({
		activeConversationId: "conversation",
		messages: [assistant()],
		queue: [],
	});
});

test("global prompts carry the quoted target even when it is outside the history", () => {
	const message: IMessage = {
		...assistant(),
		id: "reply",
		inner: { role: IRole.User, content: "What makes it cheaper?" },
		interaction: { type: "reply", target: createMessageReference(assistant()) },
	};
	const input = globalChatModelInput(message, []);
	expect(input.prompt).toContain('"message_id": "old-answer"');
	expect(input.prompt).toContain("The second option costs less.");
	expect(input.prompt.endsWith("What makes it cheaper?")).toBe(true);
	expect(message.inner.content).toBe("What makes it cheaper?");
	expect(input.history).toEqual([]);
});

test("prior reaction turns retain their meaning when included in the next history", () => {
	const prior: IMessage = {
		...assistant(),
		id: "reaction-turn",
		inner: { role: IRole.User, content: "👍" },
		interaction: reaction("remove"),
	};
	const current = {
		...prior,
		id: "next",
		interaction: undefined,
		inner: { role: IRole.User, content: "Continue" },
	};
	const input = globalChatModelInput(current, [assistant(), prior]);
	expect(input.prompt).toBe("Continue");
	expect(input.history[0]).toEqual({
		role: "Assistant",
		content: assistant().inner.content,
	});
	expect(input.history[1].role).toBe("User");
	expect(input.history[1].content).toContain('"operation": "remove"');
	expect(input.history[1].content).toContain('"emoji": "👍"');
	expect(prior.inner.content).toBe("👍");
});

test("accepted reactions persist independently of ratings and apply operations idempotently", async () => {
	await persistGlobalChatReaction(reaction(), "conversation");
	await persistGlobalChatReaction(reaction(), "conversation");
	expect(useGlobalChatStore.getState().messages[0].reactions).toEqual(["👍"]);
	expect((await globalChatDb.messages.get("old-answer"))?.reactions).toEqual([
		"👍",
	]);
	expect((await globalChatDb.messages.get("old-answer"))?.rating).toBe(1);
	await persistGlobalChatReaction(reaction("remove"), "conversation");
	expect(useGlobalChatStore.getState().messages[0].reactions).toEqual([]);
	expect((await globalChatDb.messages.get("old-answer"))?.reactions).toEqual(
		[],
	);
});

test("changing an accepted reaction updates user emojis together and retains workflow reactions", async () => {
	useGlobalChatStore.setState({
		messages: [
			{ ...assistant(), reactions: ["👍", "🎉"], workflowReactions: ["👍"] },
		],
	});
	const interaction: IChatMessageInteraction = {
		type: "reaction",
		emoji: "❤️",
		operation: "add",
		replacesEmoji: "👍",
		target: createMessageReference(assistant()),
	};
	await persistGlobalChatReaction(interaction, "conversation");
	await persistGlobalChatReaction(interaction, "conversation");
	const expected = {
		reactions: ["🎉", "❤️"],
		workflowReactions: ["👍"],
		rating: 1,
	};
	expect(useGlobalChatStore.getState().messages[0]).toMatchObject(expected);
	expect(await globalChatDb.messages.get("old-answer")).toMatchObject(expected);
	const input = globalChatModelInput(
		{
			...assistant(),
			id: "changed-reaction",
			inner: { role: IRole.User, content: "❤️" },
			interaction,
		},
		[],
	);
	expect(input.prompt).toContain('"replaces_emoji": "👍"');
	expect(input.prompt).toContain('"emoji": "❤️"');
});

test("a reaction cannot update a message in another conversation", async () => {
	await expect(persistGlobalChatReaction(reaction(), "other")).rejects.toThrow(
		"no longer available",
	);
	expect(useGlobalChatStore.getState().messages[0].reactions).toBeUndefined();
	expect(await globalChatDb.messages.get("old-answer")).toBeUndefined();
});

test("a queued replacement retains its accepted state after a later removal", async () => {
	await persistGlobalChatReaction(reaction(), "conversation");
	const interaction: IChatMessageInteraction = {
		type: "reaction",
		emoji: "❤️",
		operation: "add",
		replacesEmoji: "👍",
		target: createMessageReference(assistant()),
	};
	await persistGlobalChatReaction(interaction, "conversation");
	const store = useGlobalChatStore.getState();
	store.enqueueMessage({
		conversationId: "conversation",
		content: "❤️",
		interaction,
		reactionAccepted: true,
	});
	await persistGlobalChatReaction(
		{ ...interaction, operation: "remove", replacesEmoji: undefined },
		"conversation",
	);
	const queued = store.takeNextQueuedMessage("conversation");
	expect(queued?.interaction).toEqual(interaction);
	expect(queued?.reactionAccepted).toBe(true);
	expect(queued?.content).toBe("❤️");
	expect(useGlobalChatStore.getState().messages[0].reactions).toEqual([]);
	expect((await globalChatDb.messages.get("old-answer"))?.reactions).toEqual(
		[],
	);
});

test("capacity queues preserve target snapshots and accepted state in order", async () => {
	const store = useGlobalChatStore.getState();
	for (const operation of ["add", "remove"] as const) {
		const interaction = reaction(operation);
		await persistGlobalChatReaction(interaction, "conversation");
		store.enqueueMessage({
			conversationId: "conversation",
			content: "👍",
			interaction,
			reactionAccepted: true,
		});
	}
	expect(useGlobalChatStore.getState().messages[0].reactions).toEqual([]);
	for (const operation of ["add", "remove"] as const) {
		const entry = store.takeNextQueuedMessage("conversation");
		expect(entry?.interaction).toEqual(reaction(operation));
		expect(entry?.reactionAccepted).toBe(true);
		expect(entry?.content).toBe("👍");
	}
	expect(store.takeNextQueuedMessage("conversation")).toBeUndefined();
	expect(useGlobalChatStore.getState().messages[0].reactions).toEqual([]);
});

describe("shared user context in model history", () => {
	const request: IUserContextRequest = {
		id: "ctx",
		bot_id: "bot",
		subject_id: "me",
		conversation_id: "conversation",
		purpose: "Weather",
		fields: ["location"],
		precision: "approximate",
		audience: "direct",
		audience_member_ids: ["bot", "me"],
		created_at: 0,
		expires_at: 300_000,
	};
	const response = (decision: "once" | "session" | "deny") => ({
		request_id: "ctx",
		bot_id: "bot",
		subject_id: "me",
		conversation_id: "conversation",
		decision,
		captured_at: 1_000_000,
		expires_at: 1_300_000,
		source: { session_id: "s", platform: "web" },
		values: { location: { place: "Munich" } },
		unavailable: {},
	});
	const cardMessage = (
		decision: "once" | "session" | "deny",
		revoked = false,
	): IMessage => ({
		...assistant(),
		id: "card",
		inner: { role: IRole.Assistant, content: "" },
		userContext: {
			request,
			answer: decision,
			response: response(decision),
			...(revoked ? { revoked } : {}),
		},
	});
	const derived: IMessage = {
		...assistant(),
		id: "derived",
		inner: { role: IRole.Assistant, content: "You are near Munich." },
		userContextDerived: true,
		userContextDerivedFrom: "ctx",
	};
	const next: IMessage = {
		...assistant(),
		id: "next",
		inner: { role: IRole.User, content: "And tomorrow?" },
	};

	test("a live session shows the snapshot and keeps the derived reply", () => {
		const { history } = globalChatModelInput(
			next,
			[cardMessage("session"), derived],
			1_000_000 + 5 * 60_000,
		);
		expect(history[0]).toEqual({
			role: "Assistant",
			content: "[You asked for user context (location).]",
		});
		expect(history[1].role).toBe("User");
		expect(history[1].content).toContain(
			"captured 5 min ago and usable here for about 25 more min",
		);
		expect(history[1].content).toContain('"place":"Munich"');
		expect(history[2].content).toBe("You are near Munich.");
	});

	test("an ended session withholds values and the derived reply", () => {
		const { history } = globalChatModelInput(
			next,
			[cardMessage("session"), derived],
			1_000_000 + 31 * 60_000,
		);
		expect(history[1].content).toContain("for a session that has ended");
		expect(history[1].content).not.toContain("Munich");
		expect(history[2].content).toBe(
			"[Your reply here used shared user context and is withheld now.]",
		);
	});

	test("a revoked session is marked and a once answer is withheld", () => {
		const revoked = globalChatModelInput(
			next,
			[cardMessage("session", true), derived],
			1_000_001,
		).history;
		expect(revoked[1].content).toBe(
			"[Shared user context (location) for a session, then revoked access. The values are withheld; do not use them.]",
		);
		expect(revoked[2].content).toContain("withheld");
		const once = globalChatModelInput(
			next,
			[cardMessage("once"), derived],
			1_000_001,
		).history;
		expect(once[1].content).toBe(
			"[Shared user context (location) once. It was for that answer only and is withheld now; request it again if you need it.]",
		);
		expect(JSON.stringify(once)).not.toContain("Munich");
	});
});
