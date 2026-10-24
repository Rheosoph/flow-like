import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import Dexie from "dexie";
import type { SurfaceComponent } from "../../components/a2ui/types";
import { IRole } from "../../lib/schema/llm/history";
import { type IMessage, globalChatDb } from "./global-chat-db";
import {
	beginGlobalChatTurnSelection,
	useGlobalChatStore,
} from "./global-chat-store";

const initialState = useGlobalChatStore.getInitialState();

function startRun(runId: string, conversationId = "conversation-1") {
	const reply: IMessage = {
		id: runId,
		appId: "global",
		sessionId: conversationId,
		inner: { role: IRole.Assistant, content: "" },
		files: [],
		timestamp: 100,
	};
	useGlobalChatStore.getState().startRun({
		runId,
		conversationId,
		selection: beginGlobalChatTurnSelection(runId),
		label: runId,
		message: reply,
		sourceAttachments: [],
	});
}

function components(...ids: string[]): SurfaceComponent[] {
	return ids.map(
		(id) =>
			({
				id,
				component: { type: "text", content: { literalString: id } },
			}) as unknown as SurfaceComponent,
	);
}

const store = () => useGlobalChatStore.getState();

describe("pending components live draft", () => {
	let put: ReturnType<typeof spyOn<typeof globalChatDb.messages, "put">>;

	beforeEach(() => {
		useGlobalChatStore.setState({
			...initialState,
			activeConversationId: "conversation-1",
		});
		put = spyOn(globalChatDb.messages, "put").mockImplementation((value) =>
			Dexie.Promise.resolve(value.id ?? "persisted-message"),
		);
		startRun("run-1");
	});

	afterEach(() => {
		put.mockRestore();
		useGlobalChatStore.setState(initialState);
	});

	test("fills an empty slot and leaves nothing behind when it ends", () => {
		store().setPendingComponentsDraft("run-1", "call-1", {
			components: components("root"),
		});
		expect(store().pendingComponents).toEqual({
			components: components("root"),
			draftRequestId: "call-1",
		});
		expect(store().pendingComponentsOwnerRunId).toBe("run-1");

		store().setPendingComponentsDraft("run-1", "call-1", null);
		expect(store().pendingComponents).toBeNull();
		expect(store().pendingComponentsOwnerRunId).toBeNull();
	});

	test("gives back the staged review it covered when it ends without staging", () => {
		startRun("run-0");
		store().setPendingComponents("run-0", { components: components("old") });
		const staged = store().pendingComponents;

		store().setPendingComponentsDraft("run-1", "call-1", {
			components: components("a"),
		});
		store().setPendingComponentsDraft("run-1", "call-1", {
			components: components("a", "b"),
		});
		expect(store().pendingComponents?.components).toEqual(components("a", "b"));

		store().setPendingComponentsDraft("run-1", "call-1", null);
		expect(store().pendingComponents).toBe(staged);
		expect(store().pendingComponentsOwnerRunId).toBe("run-0");
	});

	test("a staged result supersedes the draft and the review it covered", () => {
		store().setPendingComponents("run-1", { components: components("old") });
		store().setPendingComponentsDraft("run-1", "call-1", {
			components: components("a"),
		});
		store().setPendingComponents("run-1", {
			components: components("final"),
		});
		store().setPendingComponentsDraft("run-1", "call-1", null);
		expect(store().pendingComponents).toEqual({
			components: components("final"),
		});
	});

	test("never replaces another call's live draft", () => {
		store().setPendingComponentsDraft("run-1", "call-1", {
			components: components("a"),
		});
		store().setPendingComponentsDraft("run-1", "call-2", {
			components: components("x"),
		});
		store().setPendingComponentsDraft("run-1", "call-2", null);
		expect(store().pendingComponents?.draftRequestId).toBe("call-1");
		expect(store().pendingComponents?.components).toEqual(components("a"));
	});

	test("only reaches the conversation on screen", () => {
		startRun("background-run", "conversation-2");
		store().setPendingComponentsDraft("background-run", "call-1", {
			components: components("a"),
		});
		expect(store().pendingComponents).toBeNull();
	});
});
