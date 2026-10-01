import { afterEach, beforeEach, describe, expect, spyOn, test } from "bun:test";
import Dexie from "dexie";
import type { FlowScriptWorkspaceCandidate } from "../../components/flowpilot/flowscript-workspace-candidates";
import { IRole } from "../../lib/schema/llm/history";
import { type IMessage, globalChatDb } from "./global-chat-db";
import {
	beginGlobalChatTurnSelection,
	useGlobalChatStore,
} from "./global-chat-store";
import { persistGlobalChatMessage } from "./global-chat-stream";

const initialState = useGlobalChatStore.getInitialState();

function message(
	id: string,
	sessionId = "conversation-1",
	timestamp = 100,
): IMessage {
	return {
		id,
		appId: "global",
		sessionId,
		inner: { role: IRole.Assistant, content: "Review the generated workflow." },
		files: [],
		timestamp,
	};
}

function startRun(reply: IMessage) {
	useGlobalChatStore.getState().startRun({
		runId: reply.id,
		conversationId: reply.sessionId,
		selection: beginGlobalChatTurnSelection(reply.id),
		label: reply.id,
		message: reply,
		sourceAttachments: [],
	});
}

describe("global FlowScript workspace history", () => {
	let saved: IMessage[] = [];
	let put: ReturnType<typeof spyOn<typeof globalChatDb.messages, "put">>;

	beforeEach(() => {
		useGlobalChatStore.setState({
			...initialState,
			activeConversationId: "conversation-1",
		});
		saved = [];
		put = spyOn(globalChatDb.messages, "put").mockImplementation((value) => {
			saved.push(JSON.parse(JSON.stringify(value)));
			return Dexie.Promise.resolve(value.id ?? "persisted-message");
		});
	});

	afterEach(() => {
		put.mockRestore();
		useGlobalChatStore.setState(initialState);
	});

	test("keeps complete source after the turn and restores it from message history", async () => {
		const reply = message("run-1");
		const workspace = {
			source: `// Generated workflow\n${"const value: int = 1;\n".repeat(400)}`,
			status: "queued",
		};
		startRun(reply);
		useGlobalChatStore.getState().setFlowscriptWorkspace(reply.id, workspace);
		// The outer stream replaces its own message after the nested board agent returns.
		useGlobalChatStore.getState().setRunMessage(reply.id, {
			...reply,
			inner: { ...reply.inner, content: "The workflow is awaiting approval." },
		});
		const finalized = useGlobalChatStore.getState().runs[reply.id].message;
		if (!finalized) throw new Error("The streaming reply was lost.");
		expect(finalized.flowscript_workspace).toEqual(workspace);
		await persistGlobalChatMessage(finalized);
		useGlobalChatStore.getState().commitMessage(finalized);
		useGlobalChatStore.getState().endRun(reply.id);
		expect(useGlobalChatStore.getState()).toMatchObject({
			flowscriptWorkspace: workspace,
			flowscriptWorkspaceOwnerRunId: null,
		});

		useGlobalChatStore.getState().loadConversation("other", []);
		expect(useGlobalChatStore.getState().flowscriptWorkspace).toBeNull();
		useGlobalChatStore.getState().loadConversation(reply.sessionId, saved);
		expect(useGlobalChatStore.getState().flowscriptWorkspace).toEqual(
			workspace,
		);
	});

	test("keeps concurrent runs' sources separate when either run ends", () => {
		const first = message("run-1");
		const second = message("run-2", "conversation-1", 200);
		const firstSource = { source: "const first: int = 1;", status: "queued" };
		const secondSource = { source: "const second: int = 2;", status: "queued" };
		startRun(first);
		startRun(second);
		const store = useGlobalChatStore.getState();
		store.setFlowscriptWorkspace(first.id, firstSource);
		store.setFlowscriptWorkspace(second.id, secondSource);
		store.setRunMessage(first.id, first);
		store.setRunMessage(second.id, second);
		expect(
			useGlobalChatStore.getState().runs[first.id].message
				?.flowscript_workspace,
		).toEqual(firstSource);
		expect(
			useGlobalChatStore.getState().runs[second.id].message
				?.flowscript_workspace,
		).toEqual(secondSource);
		store.setFlowscriptWorkspace(first.id, null);
		store.endRun(first.id);
		expect(useGlobalChatStore.getState().flowscriptWorkspace).toEqual(
			secondSource,
		);
		store.endRun(second.id);
		store.setFlowscriptWorkspace(first.id, firstSource);
		expect(useGlobalChatStore.getState().flowscriptWorkspace).toEqual(
			secondSource,
		);
	});

	test("background updates stay with their conversation and restore while still running", () => {
		const reply = message("run-1");
		const workspace = { source: "const retained: int = 1;", status: "queued" };
		startRun(reply);
		useGlobalChatStore.getState().loadConversation("other", []);
		useGlobalChatStore.getState().setFlowscriptWorkspace(reply.id, workspace);
		expect(useGlobalChatStore.getState().flowscriptWorkspace).toBeNull();
		useGlobalChatStore.getState().loadConversation(reply.sessionId, []);
		expect(useGlobalChatStore.getState()).toMatchObject({
			flowscriptWorkspace: workspace,
			flowscriptWorkspaceOwnerRunId: reply.id,
		});
		useGlobalChatStore.getState().loadConversation("other", []);
		useGlobalChatStore.getState().endRun(reply.id);
		expect(useGlobalChatStore.getState().flowscriptWorkspace).toBeNull();
	});

	test("replaces interrupted previews with the last complete source", () => {
		const reply = message("run-1");
		const workspace = { source: "const complete: int = 1;", status: "queued" };
		startRun(reply);
		const store = useGlobalChatStore.getState();
		store.setFlowscriptWorkspace(reply.id, workspace);
		store.setFlowscriptWorkspace(reply.id, {
			source: "event fn incomplete(",
			status: "drafting",
		});
		expect(
			useGlobalChatStore.getState().runs[reply.id].message
				?.flowscript_workspace,
		).toEqual(workspace);
		store.endRun(reply.id);
		expect(useGlobalChatStore.getState().flowscriptWorkspace).toEqual(
			workspace,
		);
	});

	test("restores prior history when a new run only produced an incomplete preview", () => {
		const workspace = { source: "const complete: int = 1;", status: "queued" };
		const history = { ...message("old"), flowscript_workspace: workspace };
		const reply = message("run-1", "conversation-1", 200);
		useGlobalChatStore.getState().loadConversation(reply.sessionId, [history]);
		startRun(reply);
		useGlobalChatStore.getState().setFlowscriptWorkspace(reply.id, {
			source: "event fn incomplete(",
			status: "drafting",
		});
		useGlobalChatStore.getState().endRun(reply.id);
		expect(useGlobalChatStore.getState().flowscriptWorkspace).toEqual(
			workspace,
		);
	});

	test("restores the latest complete source and ignores foreign or partial messages", () => {
		const workspace = { source: "const newest: int = 2;", status: "queued" };
		useGlobalChatStore.getState().loadConversation("conversation-1", [
			{
				...message("new", "conversation-1", 200),
				flowscript_workspace: workspace,
			},
			{
				...message("foreign", "other", 500),
				flowscript_workspace: {
					source: "const other: int = 3;",
					status: "queued",
				},
			},
			{
				...message("partial", "conversation-1", 400),
				flowscript_workspace: {
					source: "event fn partial(",
					status: "drafting",
				},
			},
			{
				...message("old", "conversation-1", 100),
				flowscript_workspace: {
					source: "const old: int = 1;",
					status: "queued",
				},
			},
		]);
		expect(useGlobalChatStore.getState().flowscriptWorkspace).toEqual(
			workspace,
		);
	});

	test("seeds resumed runs from their checkpointed source", () => {
		const workspace = { source: "const restored: int = 1;", status: "queued" };
		const reply = { ...message("run-1"), flowscript_workspace: workspace };
		startRun(reply);
		useGlobalChatStore.getState().setRunMessage(reply.id, message(reply.id));
		expect(
			useGlobalChatStore.getState().runs[reply.id].message
				?.flowscript_workspace,
		).toEqual(workspace);
	});

	test("redacts secrets in saved source without changing the live review", async () => {
		const workspace: FlowScriptWorkspaceCandidate = {
			source: '@secret\nconst apiKey: string = "example-sensitive-key";',
			status: "queued",
			retained_full_source: "private draft metadata",
			diagnostics: ["private diagnostic metadata"],
		};
		const reply = { ...message("run-1"), flowscript_workspace: workspace };
		await persistGlobalChatMessage(reply);
		expect(saved[0].flowscript_workspace).toEqual({
			source: '@secret\nconst apiKey: string = "";',
			status: "queued",
		});
		expect(reply.flowscript_workspace).toBe(workspace);
		expect(reply.flowscript_workspace.source).toContain(
			"example-sensitive-key",
		);
	});

	test("omits incomplete and unredactable source from persisted messages", async () => {
		for (const workspace of [
			{ source: "event fn incomplete(", status: "drafting" },
			{
				source: "@secret\nconst key: string = lookupSecret();",
				status: "queued",
			},
		]) {
			await persistGlobalChatMessage({
				...message("run-1"),
				flowscript_workspace: workspace,
			});
		}
		expect(saved).toHaveLength(2);
		for (const reply of saved)
			expect(reply.flowscript_workspace).toBeUndefined();
	});
});
