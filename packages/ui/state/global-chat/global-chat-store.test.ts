import {
	afterAll,
	beforeAll,
	beforeEach,
	describe,
	expect,
	test,
} from "bun:test";
import {
	type GlobalChatTurnSelection,
	beginGlobalChatTurnSelection,
	getGlobalChatTurnSelection,
	isGlobalChatAtRunCapacity,
	selectGlobalChatQueue,
	selectGlobalChatRuns,
	useGlobalChatStore,
} from "./global-chat-store";
import {
	clearActiveRun,
	readActiveRun,
	setActiveRun,
} from "./global-chat-stream";

const sessionValues = new Map<string, string>();
const sessionStorageStub: Storage = {
	get length() {
		return sessionValues.size;
	},
	clear: () => sessionValues.clear(),
	getItem: (key) => sessionValues.get(key) ?? null,
	key: (index) => [...sessionValues.keys()][index] ?? null,
	removeItem: (key) => sessionValues.delete(key),
	setItem: (key, value) => sessionValues.set(key, value),
};
let originalSessionStorage: Storage | undefined;

describe("global chat overlay dismissal", () => {
	beforeAll(() => {
		originalSessionStorage = globalThis.sessionStorage;
		Object.defineProperty(globalThis, "sessionStorage", {
			configurable: true,
			value: sessionStorageStub,
		});
	});

	afterAll(() => {
		if (originalSessionStorage) {
			Object.defineProperty(globalThis, "sessionStorage", {
				configurable: true,
				value: originalSessionStorage,
			});
		} else {
			Reflect.deleteProperty(globalThis, "sessionStorage");
		}
	});

	beforeEach(() => {
		sessionStorageStub.clear();
		useGlobalChatStore.setState({
			mode: "closed",
			overlayAutoOpenDismissed: false,
			provider: "bits",
			selectedModelId: "",
			reasoningEffort: "",
			activeConversationId: "conversation-1",
			messages: [],
			runs: {},
			queue: [],
			isStreaming: false,
			streamingMessages: [],
		});
	});

	/** Register a run the way the stream engine does, so per-run state has somewhere to live. */
	const startRun = (runId: string, selection: GlobalChatTurnSelection) => {
		useGlobalChatStore.getState().startRun({
			runId,
			conversationId: selection.runId === runId ? "conversation-1" : "other",
			selection,
			label: runId,
			message: null,
			sourceAttachments: [],
		});
	};

	test("keeps neutral closes eligible for a later automatic open", () => {
		const state = useGlobalChatStore.getState();
		state.openOverlay();
		state.closeOverlay();
		useGlobalChatStore.getState().openOverlayIfAllowed();

		expect(useGlobalChatStore.getState()).toMatchObject({
			mode: "overlay",
			overlayAutoOpenDismissed: false,
		});
	});

	test("does not clear a dismissal during the neutral close on the chat page", () => {
		const state = useGlobalChatStore.getState();
		state.dismissOverlay();
		state.closeOverlay();
		useGlobalChatStore.getState().openOverlayIfAllowed();

		expect(useGlobalChatStore.getState()).toMatchObject({
			mode: "closed",
			overlayAutoOpenDismissed: true,
		});
	});

	test("blocks automatic opens after the user dismisses the overlay", () => {
		const state = useGlobalChatStore.getState();
		state.openOverlay();
		state.dismissOverlay();
		useGlobalChatStore.getState().openOverlayIfAllowed();

		expect(useGlobalChatStore.getState()).toMatchObject({
			mode: "closed",
			overlayAutoOpenDismissed: true,
		});
	});

	test("allows automatic opens after renewed FlowPilot page interaction", () => {
		const state = useGlobalChatStore.getState();
		state.dismissOverlay();
		useGlobalChatStore.getState().enableOverlayAutoOpen();
		useGlobalChatStore.getState().openOverlayIfAllowed();

		expect(useGlobalChatStore.getState()).toMatchObject({
			mode: "overlay",
			overlayAutoOpenDismissed: false,
		});
	});

	test("keeps explicit opens available while automatic opens are dismissed", () => {
		useGlobalChatStore.getState().dismissOverlay();
		useGlobalChatStore.getState().openOverlay();

		expect(useGlobalChatStore.getState()).toMatchObject({
			mode: "overlay",
			overlayAutoOpenDismissed: true,
		});
	});

	test("restores the dismissal latch from session storage", () => {
		useGlobalChatStore.getState().dismissOverlay();
		expect(
			sessionStorageStub.getItem("flow-like:global-chat:auto-open-dismissed"),
		).toBe("true");

		// Simulate a freshly-created in-memory store after a hard reload.
		useGlobalChatStore.setState({
			mode: "closed",
			overlayAutoOpenDismissed: false,
		});
		useGlobalChatStore.getState().openOverlayIfAllowed();
		expect(useGlobalChatStore.getState()).toMatchObject({
			mode: "closed",
			overlayAutoOpenDismissed: true,
		});

		useGlobalChatStore.getState().enableOverlayAutoOpen();
		expect(
			sessionStorageStub.getItem("flow-like:global-chat:auto-open-dismissed"),
		).toBeNull();
	});

	test("pins one immutable provider/model/effort selection for a complete turn", () => {
		useGlobalChatStore.setState({
			provider: "codex",
			selectedModelId: "gpt-5.6-terra",
			reasoningEffort: "high",
		});
		const captured = beginGlobalChatTurnSelection("run-1");
		startRun("run-1", captured);

		expect(Object.isFrozen(captured)).toBe(true);
		expect(captured).toEqual({
			runId: "run-1",
			provider: "codex",
			selectedModelId: "gpt-5.6-terra",
			reasoningEffort: "high",
		});

		// Catalog hydration or a user picker change may update the next-turn preference while this
		// run streams; every nested specialist must continue to see the captured selection.
		useGlobalChatStore.setState({
			provider: "claude-code",
			selectedModelId: "claude-opus",
			reasoningEffort: "max",
		});
		expect(getGlobalChatTurnSelection("run-1")).toBe(captured);
		expect(
			beginGlobalChatTurnSelection("run-1", {
				provider: "github-copilot",
				selectedModelId: "different-model",
				reasoningEffort: "low",
			}),
		).toBe(captured);
	});

	test("ending a run releases only its own pinned selection", () => {
		const captured = beginGlobalChatTurnSelection("run-1", {
			provider: "codex",
			selectedModelId: "gpt-5.6-terra",
			reasoningEffort: "high",
		});
		startRun("run-1", captured);

		useGlobalChatStore.getState().endRun("stale-run");
		expect(getGlobalChatTurnSelection("run-1")).toBe(captured);

		useGlobalChatStore.getState().endRun("run-1");
		expect(useGlobalChatStore.getState().runs["run-1"]).toBeUndefined();
		expect(getGlobalChatTurnSelection("run-1")).toEqual({
			provider: "bits",
			selectedModelId: "",
			reasoningEffort: "",
		});
	});

	test("concurrent runs each pin their own selection", () => {
		const first = beginGlobalChatTurnSelection("run-1", {
			provider: "codex",
			selectedModelId: "gpt-5.6-terra",
			reasoningEffort: "high",
		});
		startRun("run-1", first);
		const second = beginGlobalChatTurnSelection("run-2", {
			provider: "claude-code",
			selectedModelId: "claude-opus",
			reasoningEffort: "max",
		});
		startRun("run-2", second);

		expect(getGlobalChatTurnSelection("run-1")).toBe(first);
		expect(getGlobalChatTurnSelection("run-2")).toBe(second);
		expect(selectGlobalChatRuns(useGlobalChatStore.getState())).toHaveLength(2);
		expect(useGlobalChatStore.getState().isStreaming).toBe(true);
	});

	test("keeps other conversations' runs alive across a conversation switch", () => {
		const selection = beginGlobalChatTurnSelection("run-1");
		startRun("run-1", selection);
		expect(useGlobalChatStore.getState().isStreaming).toBe(true);

		useGlobalChatStore.getState().loadConversation("conversation-2", []);
		expect(useGlobalChatStore.getState().runs["run-1"]).toBeDefined();
		expect(useGlobalChatStore.getState().isStreaming).toBe(false);

		useGlobalChatStore.getState().loadConversation("conversation-1", []);
		expect(useGlobalChatStore.getState().isStreaming).toBe(true);
	});

	test("queues per conversation and drains in send order", () => {
		const store = useGlobalChatStore.getState();
		store.enqueueMessage({
			conversationId: "conversation-1",
			content: "first",
		});
		store.enqueueMessage({
			conversationId: "conversation-1",
			content: "second",
		});
		store.enqueueMessage({ conversationId: "other", content: "elsewhere" });

		expect(
			selectGlobalChatQueue(useGlobalChatStore.getState()).map(
				(entry) => entry.content,
			),
		).toEqual(["first", "second"]);
		expect(
			useGlobalChatStore.getState().takeNextQueuedMessage("conversation-1")
				?.content,
		).toBe("first");
		expect(
			useGlobalChatStore.getState().takeNextQueuedMessage("conversation-1")
				?.content,
		).toBe("second");
		expect(
			useGlobalChatStore.getState().takeNextQueuedMessage("conversation-1"),
		).toBeUndefined();
		expect(useGlobalChatStore.getState().queue).toHaveLength(1);
	});

	test("reports capacity once the concurrency cap is reached", () => {
		for (let index = 0; index < 4; index++) {
			const runId = `run-${index}`;
			startRun(runId, beginGlobalChatTurnSelection(runId));
		}
		expect(isGlobalChatAtRunCapacity(useGlobalChatStore.getState())).toBe(true);
		useGlobalChatStore.getState().endRun("run-0");
		expect(isGlobalChatAtRunCapacity(useGlobalChatStore.getState())).toBe(
			false,
		);
	});

	test("a finished run only commits into the conversation it belongs to", () => {
		useGlobalChatStore.getState().appendMessage({
			id: "m1",
			appId: "global",
			sessionId: "other-conversation",
			inner: { role: "user" as never, content: "stale" },
			files: [],
			tools: [],
			actions: [],
			timestamp: Date.now(),
		});
		expect(useGlobalChatStore.getState().messages).toHaveLength(0);
	});

	test("persists the turn selection alongside the resumable run pointer", () => {
		setActiveRun("conversation-1", "run-1", {
			provider: "codex",
			selectedModelId: "gpt-5.6-terra",
			reasoningEffort: "high",
		});

		expect(readActiveRun()).toEqual({
			conversationId: "conversation-1",
			runId: "run-1",
			agentSelection: {
				provider: "codex",
				selectedModelId: "gpt-5.6-terra",
				reasoningEffort: "high",
			},
		});
		clearActiveRun("stale-run");
		expect(readActiveRun()?.runId).toBe("run-1");
		clearActiveRun("run-1");
		expect(readActiveRun()).toBeNull();
	});
});
