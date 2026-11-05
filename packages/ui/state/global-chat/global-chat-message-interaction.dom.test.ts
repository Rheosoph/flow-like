// @vitest-environment happy-dom

import "fake-indexeddb/auto";
import { beforeEach, expect, test } from "vitest";
import type { IMessage } from "../../components/interfaces/chat-default/chat-db";
import {
	type IChatMessageInteraction,
	createMessageReference,
} from "../../components/interfaces/chat-default/chat-message-interaction";
import { IRole } from "../../lib/schema/llm/history";
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
