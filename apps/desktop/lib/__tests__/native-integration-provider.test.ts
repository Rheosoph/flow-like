// @vitest-environment happy-dom

import { registerNativeActionRun } from "@flow-like/flow-like-ui/lib/native-action-result";
import type {
	IBotState,
	IBotTurn,
} from "@flow-like/flow-like-ui/state/backend-state/bot-state";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { NativeIntegrationProvider } from "../../components/native-integration-provider";
import type {
	NativePendingAction,
	NativeSnapshot,
} from "../native-integration";

const mocks = vi.hoisted(() => ({
	push: vi.fn(),
	invoke: vi.fn(),
	loadSnapshot: vi.fn(),
	resolveNotificationIcons: vi.fn(),
	executeWithResult: vi.fn(),
	setDraft: vi.fn(),
	setChatState: vi.fn(),
	cancelChatRun: vi.fn(async () => {}),
	error: vi.fn(),
	ready: true,
	pathname: "/library",
	query: "",
	webOrigin: undefined as string | undefined,
	auth: {
		isLoading: false,
		isAuthenticated: true,
		user: { profile: { sub: "user-a" } },
	},
	backend: {
		profile: { id: "profile-a" } as { id: string } | undefined,
		botState: undefined as IBotState | undefined,
		userState: { getProfile: vi.fn() },
		appState: { getApp: vi.fn(async () => ({ id: "app" })) },
		eventState: { getEventAuthoritative: vi.fn() },
	},
	engine: {
		getActiveExecutions: vi.fn(() => []),
		subscribeToGlobalUpdates: vi.fn(() => () => {}),
		executeEvent: vi.fn(async () => {}),
		cancelTask: vi.fn(async () => {}),
	},
	listeners: new Map<string, (event: { payload: unknown }) => void>(),
}));

vi.mock("@flow-like/flow-like-ui/lib/api-url", () => ({
	getApiOrigin: () => "https://api.flow-like.com",
}));
vi.mock("@flow-like/flow-like-ui/lib/recent-apps", () => ({
	readRecentApps: () => [],
	RECENT_APPS_CHANGED: "recent-apps-changed",
}));
vi.mock("@flow-like/flow-like-ui/state/backend-state", () => ({
	useBackend: () => mocks.backend,
	useBackendReady: () => mocks.ready,
}));
vi.mock("@flow-like/flow-like-ui/state/execution-engine-context", () => ({
	useExecutionEngine: () => mocks.engine,
}));
vi.mock("@flow-like/flow-like-ui/state/global-chat/global-chat-store", () => ({
	useGlobalChatStore: {
		getState: () => ({ setDraft: mocks.setDraft }),
		setState: mocks.setChatState,
	},
}));
vi.mock("@flow-like/flow-like-ui/state/global-chat/global-chat-stream", () => ({
	cancelGlobalChatRun: mocks.cancelChatRun,
}));
vi.mock("@tauri-apps/api/core", () => ({
	isTauri: () => true,
	invoke: mocks.invoke,
}));
vi.mock("@tauri-apps/api/event", () => ({
	listen: async (
		name: string,
		callback: (event: { payload: unknown }) => void,
	) => {
		mocks.listeners.set(name, callback);
		return () => mocks.listeners.delete(name);
	},
}));
vi.mock("@tanstack/react-query", async (importOriginal) => {
	const actual = await importOriginal<typeof import("@tanstack/react-query")>();
	return {
		...actual,
		useQuery: (options: Parameters<typeof actual.useQuery>[0]) =>
			options.queryKey[0] === "native-web-origin"
				? { data: mocks.webOrigin }
				: actual.useQuery(options),
	};
});
vi.mock("next/navigation", () => ({
	useRouter: () => ({ push: mocks.push }),
	usePathname: () => mocks.pathname,
	useSearchParams: () => new URLSearchParams(mocks.query),
}));
vi.mock("react-oidc-context", () => ({ useAuth: () => mocks.auth }));
vi.mock("sonner", () => ({ toast: { error: mocks.error } }));
vi.mock("../native-app-icons", () => ({
	createNativeAppIconPublisher: () => ({
		sync: async () => {},
		dispose: () => {},
	}),
}));
vi.mock("../native-notification-icons", () => ({
	createNativeNotificationIconResolver: () => ({
		resolve: mocks.resolveNotificationIcons,
		dispose: () => {},
	}),
}));
vi.mock("../../components/native-event-result-dialog", () => ({
	NativeEventResultDialog: () => null,
}));
vi.mock("../native-event-execution", () => ({
	executeNativeEventWithResult: mocks.executeWithResult,
	NativeEventInteractionRequired: class extends Error {},
}));
vi.mock("../../components/native-mcp-run-dialog", () => ({
	NativeMcpRunDialog: () => null,
}));
vi.mock("../native-integration", async (importOriginal) => ({
	...(await importOriginal<typeof import("../native-integration")>()),
	loadNativeSnapshot: mocks.loadSnapshot,
}));

const scope = (user = "user-a") =>
	JSON.stringify(["https://api.flow-like.com", "profile-a", user]);
const request = (
	id: string,
	action: NativePendingAction["action"] = { kind: "flowpilot" },
	user = "user-a",
): NativePendingAction => ({ id, scope: scope(user), action });
function deferred<T>() {
	let resolve!: (value: T) => void;
	const promise = new Promise<T>((done) => {
		resolve = done;
	});
	return { promise, resolve };
}

function prepareResultEvent() {
	mocks.backend.userState.getProfile.mockResolvedValue({
		id: "profile-a",
		apps: [{ app_id: "app" }],
	});
	mocks.backend.eventState.getEventAuthoritative.mockResolvedValue({
		id: "event",
		node_id: "node",
		name: "Calculate",
		active: true,
		event_type: "api",
		inputs: [],
		config: Array.from(
			new TextEncoder().encode(
				JSON.stringify({
					native_integration: { enabled: true, surfaces: ["shortcuts"] },
				}),
			),
		),
	});
}

let root: Root;
let container: HTMLDivElement;
let queue: NativePendingAction[];
let queryClient: QueryClient;

async function render() {
	await act(async () =>
		root.render(
			createElement(
				QueryClientProvider,
				{ client: queryClient },
				createElement(NativeIntegrationProvider),
			),
		),
	);
	await flushQueries();
}
async function flushQueries() {
	await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
}
async function notify() {
	await act(async () =>
		mocks.listeners.get("native-action")?.({ payload: {} }),
	);
}

beforeEach(() => {
	vi.clearAllMocks();
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	vi.spyOn(navigator, "userAgent", "get").mockReturnValue("iPhone");
	vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
	sessionStorage.clear();
	mocks.listeners.clear();
	mocks.ready = true;
	mocks.pathname = "/library";
	mocks.query = "";
	mocks.webOrigin = undefined;
	mocks.auth.isLoading = false;
	mocks.auth.isAuthenticated = true;
	mocks.auth.user.profile.sub = "user-a";
	mocks.backend.profile = { id: "profile-a" };
	mocks.backend.botState = undefined;
	mocks.backend.userState.getProfile.mockResolvedValue({ id: "profile-a" });
	mocks.loadSnapshot.mockImplementation(() => new Promise(() => {}));
	mocks.resolveNotificationIcons.mockResolvedValue({});
	queue = [];
	queryClient = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	mocks.invoke.mockImplementation(
		async (command: string, args?: { id: string; scope: string }) => {
			if (command === "native_ack_action")
				queue = queue.filter(
					(item) => item.id !== args?.id || item.scope !== args?.scope,
				);
			if (command === "native_pending_actions") return [...queue];
			if (command === "native_automation_current") return true;
		},
	);
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	queryClient.clear();
	container.remove();
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
});

test("cold launch opens queued FlowPilot without waiting for widget API data", async () => {
	queue.push(request("cold"));
	await render();
	expect(mocks.loadSnapshot).toHaveBeenCalledOnce();
	expect(mocks.push).toHaveBeenCalledExactlyOnceWith("/bots");
	expect(mocks.error).not.toHaveBeenCalled();
});

test("warm native callbacks open voice FlowPilot while a snapshot refresh is pending", async () => {
	await render();
	queue.push(request("warm", { kind: "flowpilot", voice: true }));
	await notify();
	expect(mocks.push).toHaveBeenCalledExactlyOnceWith("/bots?voice=1");
});

test("launch delivery waits for authentication and backend readiness", async () => {
	mocks.ready = false;
	mocks.auth.isLoading = true;
	queue.push(request("cold"));
	await render();
	expect(mocks.invoke).not.toHaveBeenCalledWith("native_pending_actions");
	mocks.ready = true;
	await render();
	expect(mocks.invoke).not.toHaveBeenCalledWith("native_pending_actions");
	mocks.auth.isLoading = false;
	await render();
	expect(mocks.push).toHaveBeenCalledExactlyOnceWith("/bots");
});

test("cold launch waits for the observed profile read without a navigation rerender", async () => {
	const profile = deferred<{ id: string }>();
	mocks.backend.profile = undefined;
	mocks.backend.userState.getProfile.mockReturnValue(profile.promise);
	queue.push(
		request("profile-hydration", { kind: "flowpilot", prompt: "Hello" }),
	);
	await render();
	expect(mocks.invoke).not.toHaveBeenCalledWith("native_pending_actions");
	expect(mocks.invoke).not.toHaveBeenCalledWith("native_clear_snapshot");
	expect(mocks.loadSnapshot).not.toHaveBeenCalled();
	await act(async () => profile.resolve({ id: "profile-a" }));
	await flushQueries();
	expect(mocks.push).toHaveBeenCalledExactlyOnceWith("/bots");
	expect(mocks.setDraft).toHaveBeenCalledWith(
		expect.objectContaining({
			prompt: "Hello",
			nativeScope: scope(),
		}),
	);
	expect(mocks.backend.profile).toBeUndefined();
	expect(mocks.invoke).not.toHaveBeenCalledWith("native_clear_snapshot");
	expect(mocks.error).not.toHaveBeenCalled();
});

test("cached profile data cannot drain actions before a fresh startup read", async () => {
	const profile = deferred<{ id: string }>();
	queryClient.setQueryData([mocks.backend.userState.getProfile.name], {
		id: "old-profile",
	});
	mocks.backend.userState.getProfile.mockReturnValue(profile.promise);
	queue.push(request("fresh-profile"));
	await render();
	expect(mocks.invoke).not.toHaveBeenCalledWith("native_pending_actions");
	await act(async () => profile.resolve({ id: "profile-a" }));
	await flushQueries();
	expect(mocks.push).toHaveBeenCalledExactlyOnceWith("/bots");
	expect(mocks.invoke).not.toHaveBeenCalledWith("native_clear_snapshot");
});

test("temporary authentication hydration preserves the same account's pending actions", async () => {
	await render();
	mocks.auth.isLoading = true;
	mocks.auth.isAuthenticated = false;
	await render();
	queue.push(request("auth-restored"));
	expect(mocks.invoke).not.toHaveBeenCalledWith("native_clear_snapshot");
	mocks.auth.isLoading = false;
	mocks.auth.isAuthenticated = true;
	await render();
	expect(mocks.push).toHaveBeenCalledExactlyOnceWith("/bots");
	expect(mocks.invoke).not.toHaveBeenCalledWith("native_clear_snapshot");
});

test("route changes preserve an already consumed batch of native actions", async () => {
	const batch = deferred<NativePendingAction[]>();
	mocks.invoke.mockImplementation(async (command: string) => {
		if (command === "native_pending_actions") return batch.promise;
	});
	await render();
	mocks.pathname = "/notifications";
	mocks.query = "read=1";
	mocks.webOrigin = "https://app.flow-like.com";
	await render();
	await act(async () => batch.resolve([request("pending")]));
	expect(mocks.push).toHaveBeenCalledExactlyOnceWith("/bots");
	expect(
		mocks.invoke.mock.calls.filter(
			([name]) => name === "native_pending_actions",
		),
	).toHaveLength(1);
});

test("file-only shares open FlowPilot immediately and survive its route change", async () => {
	const file = deferred<{ bytes: number[]; name: string; mimeType: string }>();
	queue.push(
		request("files", { kind: "share", files: ["/shared/report.pdf"] }),
	);
	mocks.invoke.mockImplementation(
		async (command: string, args?: { id: string; scope: string }) => {
			if (command === "native_ack_action")
				queue = queue.filter(
					(item) => item.id !== args?.id || item.scope !== args?.scope,
				);
			if (command === "native_pending_actions") return [...queue];
			if (command === "native_read_shared_file") return file.promise;
		},
	);
	await render();
	expect(mocks.push).toHaveBeenCalledExactlyOnceWith("/bots");
	expect(mocks.setDraft).not.toHaveBeenCalled();
	mocks.pathname = "/bots";
	await render();
	await act(async () =>
		file.resolve({
			bytes: [37, 80, 68, 70],
			name: "report.pdf",
			mimeType: "application/pdf",
		}),
	);
	expect(mocks.invoke).toHaveBeenCalledWith("native_read_shared_file", {
		path: "/shared/report.pdf",
	});
	expect(mocks.setDraft).toHaveBeenCalledOnce();
	const draft = mocks.setDraft.mock.calls[0][0];
	expect(draft.prompt).toBe("");
	expect(draft.nativeScope).toBe(scope());
	expect(draft.files[0]).toBeInstanceOf(File);
	expect(draft.files[0].name).toBe("report.pdf");
	expect(draft.files[0].type).toBe("application/pdf");
	expect(
		Array.from(new Uint8Array(await draft.files[0].arrayBuffer())),
	).toEqual([37, 80, 68, 70]);
});

test("an action arriving during file delivery is drained as soon as that batch completes", async () => {
	const file = deferred<{ bytes: number[]; name: string }>();
	queue.push(
		request("files", { kind: "share", files: ["/shared/report.txt"] }),
	);
	mocks.invoke.mockImplementation(
		async (command: string, args?: { id: string; scope: string }) => {
			if (command === "native_ack_action")
				queue = queue.filter(
					(item) => item.id !== args?.id || item.scope !== args?.scope,
				);
			if (command === "native_pending_actions") return [...queue];
			if (command === "native_read_shared_file") return file.promise;
		},
	);
	await render();
	queue.push(request("next", { kind: "open_inbox" }));
	await notify();
	expect(mocks.push).toHaveBeenCalledTimes(1);
	await act(async () => file.resolve({ bytes: [65], name: "report.txt" }));
	expect(mocks.push).toHaveBeenLastCalledWith("/notifications");
});

test("account changes discard in-flight shared files before creating a chat draft", async () => {
	const file = deferred<{ bytes: number[]; name: string }>();
	queue.push(
		request("files", { kind: "share", files: ["/shared/report.txt"] }),
	);
	mocks.invoke.mockImplementation(
		async (command: string, args?: { id: string; scope: string }) => {
			if (command === "native_ack_action")
				queue = queue.filter(
					(item) => item.id !== args?.id || item.scope !== args?.scope,
				);
			if (command === "native_pending_actions") return [...queue];
			if (command === "native_read_shared_file") return file.promise;
		},
	);
	await render();
	mocks.auth.user.profile.sub = "user-b";
	await render();
	await act(async () => file.resolve({ bytes: [65], name: "report.txt" }));
	expect(mocks.invoke).toHaveBeenCalledWith("native_clear_snapshot");
	expect(mocks.setDraft).not.toHaveBeenCalled();
	expect(mocks.push).toHaveBeenCalledTimes(1);
});

test("replayed actions are deduplicated even when webview session storage is unavailable", async () => {
	vi.spyOn(Storage.prototype, "getItem").mockImplementation(() => {
		throw new Error("Unavailable");
	});
	vi.spyOn(Storage.prototype, "setItem").mockImplementation(() => {
		throw new Error("Unavailable");
	});
	queue.push(request("repeat"));
	await render();
	queue.push(request("repeat"));
	await notify();
	expect(mocks.push).toHaveBeenCalledExactlyOnceWith("/bots");
});

test("current page publication updates without restarting native action delivery", async () => {
	const snapshot: NativeSnapshot = {
		version: 1,
		scope: scope(),
		generatedAt: new Date().toISOString(),
		staleAt: new Date(Date.now() + 3_600_000).toISOString(),
		expiresAt: new Date(Date.now() + 3_600_000).toISOString(),
		sections: [],
		events: [],
		apps: [],
	};
	mocks.loadSnapshot.mockResolvedValue(snapshot);
	await render();
	mocks.pathname = "/bots";
	mocks.query = "voice=1";
	await render();
	expect(mocks.loadSnapshot).toHaveBeenCalledOnce();
	expect(
		mocks.invoke.mock.calls.filter(
			([name]) => name === "native_pending_actions",
		),
	).toHaveLength(1);
	expect(
		mocks.invoke.mock.calls.filter(
			([name]) => name === "native_publish_snapshot",
		),
	).toHaveLength(2);
});

function notificationSnapshot(user = "user-a"): NativeSnapshot {
	return {
		version: 1,
		scope: scope(user),
		generatedAt: new Date().toISOString(),
		staleAt: new Date(Date.now() + 3_600_000).toISOString(),
		expiresAt: new Date(Date.now() + 3_600_000).toISOString(),
		sections: [
			{
				kind: "inbox",
				title: "Inbox",
				state: "ready",
				items: [
					{
						id: "notice",
						title: "Review",
						action: { kind: "open_inbox", appId: "app" },
					},
				],
			},
		],
		events: [],
		apps: [],
	};
}

function botSnapshot(): NativeSnapshot {
	return {
		...notificationSnapshot(),
		bots: [
			{
				id: "bot",
				title: "Scout",
				local: false,
				encrypted: false,
				enabled: true,
				surfaces: [],
				action: { kind: "open_bot", botId: "bot" },
			},
		],
		sections: [
			{ kind: "bot_tasks", title: "Bot tasks", state: "ready", items: [] },
		],
	};
}

function latestBotItems() {
	const publication = mocks.invoke.mock.calls
		.filter(([command]) => command === "native_publish_snapshot")
		.at(-1)?.[1].snapshot as NativeSnapshot | undefined;
	return publication?.sections.find((section) => section.kind === "bot_tasks")
		?.items;
}

const queuedTurn: IBotTurn = {
	id: "turn",
	bot_id: "bot",
	conversation_id: "room",
	status: "queued",
};

test("fresh catalog status replaces an older queued status before publication", async () => {
	const snapshot = deferred<NativeSnapshot>();
	mocks.backend.botState = {
		getMobileStatus: vi.fn(async () => [queuedTurn]),
	} as unknown as IBotState;
	mocks.loadSnapshot.mockReturnValue(snapshot.promise);
	await render();
	await act(async () => {
		mocks.loadSnapshot.mock.calls[0][7]?.([{ ...queuedTurn, status: "done" }]);
		snapshot.resolve(botSnapshot());
	});
	expect(latestBotItems()).toEqual([]);
});

test("a pending status response cannot restore a turn completed by a newer event", async () => {
	const status = deferred<IBotTurn[]>();
	let deliver: (turn: IBotTurn) => void = () => {};
	mocks.backend.botState = {
		getMobileStatus: vi.fn(() => status.promise),
		turns: {
			list: async () => [],
			subscribe: async (listener: (turn: IBotTurn) => void) => {
				deliver = listener;
				return () => {};
			},
		},
	} as unknown as IBotState;
	mocks.loadSnapshot.mockResolvedValue(botSnapshot());
	await render();
	await act(async () => deliver({ ...queuedTurn, status: "done" }));
	await act(async () => status.resolve([queuedTurn]));
	expect(latestBotItems()).toEqual([]);
});

test("an older status poll cannot undo a newer empty catalog read", async () => {
	const status = deferred<IBotTurn[]>();
	mocks.backend.botState = {
		getMobileStatus: vi.fn(() => status.promise),
	} as unknown as IBotState;
	mocks.loadSnapshot.mockImplementation(async (...args) => {
		args[7]?.([]);
		return botSnapshot();
	});
	await render();
	await act(async () => status.resolve([queuedTurn]));
	expect(latestBotItems()).toEqual([]);
});

test("a late initial host read cannot restore turns absent from authoritative status", async () => {
	const local = deferred<IBotTurn[]>();
	mocks.backend.botState = {
		getMobileStatus: async () => [],
		turns: {
			list: () => local.promise,
			subscribe: async () => () => {},
		},
	} as unknown as IBotState;
	mocks.loadSnapshot.mockResolvedValue(botSnapshot());
	await render();
	await act(async () => local.resolve([queuedTurn]));
	expect(latestBotItems()).toEqual([]);
});

test("publishes usable widgets and handles actions before notification images finish", async () => {
	const icons = deferred<Record<string, { text: string }>>();
	mocks.resolveNotificationIcons.mockReturnValue(icons.promise);
	mocks.loadSnapshot.mockImplementation(
		async (
			...args: Parameters<
				typeof import("../native-integration").loadNativeSnapshot
			>
		) => {
			args[5]?.([{ id: "notice", icon: "💬" }]);
			return notificationSnapshot();
		},
	);
	queue.push(request("early", { kind: "open_inbox" }));
	await render();
	const publications = () =>
		mocks.invoke.mock.calls
			.filter(([name]) => name === "native_publish_snapshot")
			.map(([, args]) => args.snapshot as NativeSnapshot);
	expect(mocks.push).toHaveBeenCalledWith("/notifications");
	expect(publications()).toHaveLength(1);
	expect(publications()[0].sections[0].items[0].icon).toBeUndefined();
	await act(async () => icons.resolve({ notice: { text: "💬" } }));
	expect(publications()).toHaveLength(2);
	expect(publications()[1].sections[0].items[0].icon).toEqual({ text: "💬" });
});

test("discards late notification artwork after switching accounts", async () => {
	const icons = deferred<Record<string, { text: string }>>();
	mocks.resolveNotificationIcons.mockReturnValue(icons.promise);
	mocks.loadSnapshot.mockImplementation(
		async (
			...args: Parameters<
				typeof import("../native-integration").loadNativeSnapshot
			>
		) => {
			args[5]?.([{ id: "notice", icon: "💬" }]);
			return notificationSnapshot();
		},
	);
	await render();
	mocks.loadSnapshot.mockImplementation(() => new Promise(() => {}));
	mocks.auth.user.profile.sub = "user-b";
	await render();
	await act(async () => icons.resolve({ notice: { text: "💬" } }));
	expect(mocks.invoke).toHaveBeenCalledWith("native_clear_snapshot");
	expect(
		mocks.invoke.mock.calls.filter(
			([name]) => name === "native_publish_snapshot",
		),
	).toHaveLength(1);
});

test("failed acknowledgement retries delivery without opening the action twice", async () => {
	let attempts = 0;
	queue.push(request("ack-retry"));
	mocks.invoke.mockImplementation(
		async (command: string, args?: { id: string }) => {
			if (command === "native_pending_actions") return [...queue];
			if (command === "native_ack_action") {
				if (++attempts === 1) throw new Error("Bridge unavailable");
				queue = queue.filter((item) => item.id !== args?.id);
			}
		},
	);
	await render();
	expect(queue).toHaveLength(1);
	await notify();
	expect(queue).toHaveLength(0);
	expect(mocks.push).toHaveBeenCalledExactlyOnceWith("/bots");
	expect(attempts).toBe(2);
});

test("an accepted Event is acknowledged before its typed response completes", async () => {
	const result = deferred<{ text: string; json: string }>();
	mocks.executeWithResult.mockReturnValue(result.promise);
	prepareResultEvent();
	const action = {
		...request("typed", { kind: "run_event", appId: "app", eventId: "event" }),
		responseMode: "result" as const,
		responseDeadline: new Date(Date.now() + 90_000).toISOString(),
	};
	queue.push(action);
	await render();
	expect(queue).toHaveLength(0);
	expect(
		mocks.invoke.mock.calls.filter(
			([command]) => command === "native_automation_current",
		),
	).toHaveLength(0);
	expect(mocks.executeWithResult).toHaveBeenCalledOnce();
	expect(
		mocks.invoke.mock.calls.some(
			([command]) => command === "native_complete_action",
		),
	).toBe(false);
	expect(mocks.push).not.toHaveBeenCalled();
	await act(async () => result.resolve({ text: "", json: '{"total":42}' }));
	expect(mocks.invoke).toHaveBeenCalledWith("native_complete_action", {
		result: {
			id: "typed",
			scope: scope(),
			status: "success",
			text: "",
			json: '{"total":42}',
		},
	});
});

test("cancelling a typed Event stops its task and discards late output", async () => {
	prepareResultEvent();
	const output = deferred<{ text: string }>();
	mocks.executeWithResult.mockReturnValue(output.promise);
	const action = {
		...request("cancel-event", {
			kind: "run_event",
			appId: "app",
			eventId: "event",
		}),
		automation: true,
		responseMode: "text" as const,
		responseDeadline: new Date(Date.now() + 90_000).toISOString(),
	};
	queue.push(action);
	await render();
	const execution = mocks.executeWithResult.mock.calls[0][0];
	expect(execution.isCurrent()).toBe(true);
	await act(async () =>
		window.dispatchEvent(
			new CustomEvent("flow-like-native-action-cancel", {
				detail: { id: action.id, scope: scope("user-b") },
			}),
		),
	);
	expect(mocks.engine.cancelTask).not.toHaveBeenCalled();
	await act(async () =>
		window.dispatchEvent(
			new CustomEvent("flow-like-native-action-cancel", {
				detail: { id: action.id, scope: action.scope },
			}),
		),
	);
	expect(mocks.engine.cancelTask).toHaveBeenCalledExactlyOnceWith(
		"native-cancel-event",
	);
	expect(execution.isCurrent()).toBe(false);
	await act(async () => output.resolve({ text: "Late answer" }));
	expect(
		mocks.invoke.mock.calls.filter(
			([command]) => command === "native_complete_action",
		),
	).toHaveLength(0);
});

test("cancellation stops only the matching chat run and removes its queued drafts", async () => {
	const action = {
		...request("cancel-chat", { kind: "flowpilot", text: "Summarize my work" }),
		automation: true,
		responseMode: "text" as const,
		responseDeadline: new Date(Date.now() + 90_000).toISOString(),
	};
	const other = { ...request("other-chat"), responseMode: "text" as const };
	queue.push(action);
	await render();
	expect(mocks.setDraft).toHaveBeenCalledOnce();
	registerNativeActionRun(action, "answer-cancel-chat");
	registerNativeActionRun(other, "answer-other-chat");
	await act(async () =>
		window.dispatchEvent(
			new CustomEvent("flow-like-native-action-cancel", {
				detail: { id: action.id, scope: action.scope },
			}),
		),
	);
	expect(mocks.cancelChatRun).toHaveBeenCalledExactlyOnceWith(
		"answer-cancel-chat",
	);
	const same = { prompt: "cancel me", nativeRequest: action };
	const keep = { prompt: "keep me", nativeRequest: other };
	const update = mocks.setChatState.mock.calls[0][0];
	expect(
		update({ draft: same, nativeDrafts: [keep, same], queue: [same, keep] }),
	).toEqual({
		draft: keep,
		nativeDrafts: [],
		queue: [keep],
	});
});

test("switching account cancels an accepted Event before it can return output", async () => {
	prepareResultEvent();
	const output = deferred<{ text: string }>();
	mocks.executeWithResult.mockReturnValue(output.promise);
	queue.push({
		...request("switch-event", {
			kind: "run_event",
			appId: "app",
			eventId: "event",
		}),
		automation: true,
		responseMode: "text",
		responseDeadline: new Date(Date.now() + 90_000).toISOString(),
	});
	await render();
	mocks.auth.user.profile.sub = "user-b";
	await render();
	expect(mocks.engine.cancelTask).toHaveBeenCalledWith("native-switch-event");
	expect(mocks.executeWithResult.mock.calls[0][0].isCurrent()).toBe(false);
	await act(async () => output.resolve({ text: "Previous account result" }));
	expect(
		mocks.invoke.mock.calls.filter(
			([command]) => command === "native_complete_action",
		),
	).toHaveLength(0);
});

test("focus and interval refreshes reuse a recent snapshot and one scope-bound event catalog", async () => {
	mocks.loadSnapshot.mockResolvedValue(notificationSnapshot());
	const intervals = vi.spyOn(window, "setInterval");
	let now = 1_000_000;
	vi.spyOn(Date, "now").mockImplementation(() => now);
	const trigger = async (fire: () => void) => {
		await act(async () => fire());
		await flushQueries();
	};
	await render();
	const tick = intervals.mock.calls
		.filter(([, delay]) => delay === 60_000)
		.at(-1)?.[0] as () => void;
	expect(mocks.loadSnapshot).toHaveBeenCalledOnce();
	now += 30_000;
	await trigger(() => window.dispatchEvent(new Event("focus")));
	await trigger(tick);
	expect(mocks.loadSnapshot).toHaveBeenCalledOnce();
	now += 61_000;
	await trigger(tick);
	expect(mocks.loadSnapshot).toHaveBeenCalledOnce();
	await trigger(() => window.dispatchEvent(new Event("focus")));
	expect(mocks.loadSnapshot).toHaveBeenCalledTimes(2);
	await trigger(() => window.dispatchEvent(new Event("recent-apps-changed")));
	expect(mocks.loadSnapshot).toHaveBeenCalledTimes(3);
	now += 10 * 60_000;
	await trigger(tick);
	expect(mocks.loadSnapshot).toHaveBeenCalledTimes(4);
	const catalogs = mocks.loadSnapshot.mock.calls.map((args) => args[6]);
	expect(catalogs[0]).toBeInstanceOf(Map);
	expect(new Set(catalogs).size).toBe(1);
});
