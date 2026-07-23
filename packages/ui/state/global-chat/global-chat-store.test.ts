import {
	afterAll,
	beforeAll,
	beforeEach,
	describe,
	expect,
	test,
} from "bun:test";
import {
	beginGlobalChatTurnSelection,
	endGlobalChatTurnSelection,
	getGlobalChatTurnSelection,
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
			activeTurnSelection: null,
			isStreaming: false,
		});
	});

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
		expect(getGlobalChatTurnSelection()).toBe(captured);
		expect(
			beginGlobalChatTurnSelection("run-1", {
				provider: "github-copilot",
				selectedModelId: "different-model",
				reasoningEffort: "low",
			}),
		).toBe(captured);
	});

	test("only the owning run can release a pinned turn selection", () => {
		const captured = beginGlobalChatTurnSelection("run-1", {
			provider: "codex",
			selectedModelId: "gpt-5.6-terra",
			reasoningEffort: "high",
		});
		endGlobalChatTurnSelection("stale-run");
		expect(getGlobalChatTurnSelection()).toBe(captured);

		endGlobalChatTurnSelection("run-1");
		expect(useGlobalChatStore.getState().activeTurnSelection).toBeNull();
		expect(getGlobalChatTurnSelection()).toEqual({
			provider: "bits",
			selectedModelId: "",
			reasoningEffort: "",
		});
	});

	test("rejects a second run that tries to replace an active turn selection", () => {
		beginGlobalChatTurnSelection("run-1", {
			provider: "codex",
			selectedModelId: "gpt-5.6-terra",
			reasoningEffort: "high",
		});

		expect(() =>
			beginGlobalChatTurnSelection("run-2", {
				provider: "claude-code",
				selectedModelId: "claude-opus",
				reasoningEffort: "max",
			}),
		).toThrow("run 'run-1' still owns the model selection");
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
