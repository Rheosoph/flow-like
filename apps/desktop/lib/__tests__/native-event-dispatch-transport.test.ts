import { beforeEach, expect, test, vi } from "vitest";
const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	fetcher: vi.fn(),
	stream: vi.fn(),
	consent: vi.fn(),
}));
vi.mock("@flow-like/flow-like-ui", async (original) => ({
	...(await original<typeof import("@flow-like/flow-like-ui")>()),
	checkOAuthTokens: async () => ({
		tokens: {},
		requiredProviders: [],
		missingProviders: [],
	}),
	extractOAuthRequirementsFromBoard: () => ({
		requires_local_execution: false,
	}),
}));
vi.mock("@flow-like/flow-like-ui/lib/device-bridge", () => ({
	cancelDeviceCommands: vi.fn(),
	withDeviceCommandBridge: (
		_context: unknown,
		cb: unknown,
		run: (cb: unknown) => Promise<unknown>,
	) => run(cb),
}));
vi.mock("@tauri-apps/api/core", async (original) => ({
	...(await original<typeof import("@tauri-apps/api/core")>()),
	invoke: mocks.invoke,
	Channel: class {
		onmessage?: (events: unknown) => void;
	},
}));
vi.mock("../api", () => ({
	fetcher: mocks.fetcher,
	streamFetcher: mocks.stream,
}));
vi.mock("../oauth-db", () => ({
	oauthConsentStore: { getConsentedProviderIds: mocks.consent },
	oauthTokenStore: {},
}));
vi.mock("../oauth-service", () => ({
	oauthService: { refreshToken: vi.fn() },
}));
vi.mock("../../components/rpa", () => ({
	ensureRpaSystemPermissions: vi.fn(),
	requestRpaAutomationConsent: vi.fn(),
}));
import { EventState } from "../../components/tauri-provider/event-state";
function fixture() {
	const backend = {
		profile: { id: "profile-a" },
		auth: { isAuthenticated: true, user: { access_token: "token-a" } },
		isOffline: vi.fn(async () => true),
		boardState: {
			getBoard: vi.fn(async () => ({ nodes: {}, layers: {}, variables: {} })),
		},
	};
	const state = new EventState(backend as never);
	vi.spyOn(state, "getEvent").mockResolvedValue({
		id: "event",
		board_id: "board",
		execution_mode: "Local",
	} as never);
	return { backend, state };
}
beforeEach(() => {
	vi.clearAllMocks();
	mocks.consent.mockResolvedValue(new Set());
	mocks.invoke.mockResolvedValue({ id: "run" });
});

test("local Event dispatch rechecks identity after asynchronous OAuth consent preparation", async () => {
	const consent = Promise.withResolvers<Set<string>>();
	const entered = Promise.withResolvers<void>();
	mocks.consent.mockImplementation(() => {
		entered.resolve();
		return consent.promise;
	});
	const { state, backend } = fixture();
	const guard = () => {
		if (backend.profile.id !== "profile-a")
			throw new Error("Native identity changed");
	};
	const pending = state.executeEvent(
		"app",
		"event",
		{ id: "node" },
		false,
		undefined,
		undefined,
		false,
		undefined,
		guard,
	);
	await entered.promise;
	backend.profile.id = "profile-b";
	backend.auth.user.access_token = "token-b";
	consent.resolve(new Set());
	await expect(pending).rejects.toThrow("Native identity changed");
	expect(mocks.invoke).not.toHaveBeenCalled();
	expect(mocks.stream).not.toHaveBeenCalled();
});

test("valid native local dispatch still uses the ordinary Event command and current token", async () => {
	const { state } = fixture();
	const guard = vi.fn();
	await state.executeEvent(
		"app",
		"event",
		{ id: "node", payload: { question: "Hello" } },
		false,
		undefined,
		undefined,
		false,
		undefined,
		guard,
	);
	expect(mocks.invoke).toHaveBeenCalledWith(
		"execute_event",
		expect.objectContaining({
			appId: "app",
			eventId: "event",
			token: "token-a",
			payload: { id: "node", payload: { question: "Hello" } },
		}),
	);
	expect(guard).toHaveBeenCalled();
});

test("an expired remote Event never opens the SSE request", async () => {
	const { state } = fixture();
	await expect(
		state.executeEventRemote(
			"app",
			"event",
			{ id: "node" },
			false,
			undefined,
			undefined,
			undefined,
			() => {
				throw new Error("Native request expired");
			},
		),
	).rejects.toThrow("Native request expired");
	expect(mocks.stream).not.toHaveBeenCalled();
});

test("an expired MCP operation never sends a tool request", async () => {
	const { state } = fixture();
	await expect(
		state.invokeMcp("app", "event", "tools/call", { name: "send" }, () => {
			throw new Error("Native request expired");
		}),
	).rejects.toThrow("Native request expired");
	expect(mocks.fetcher).not.toHaveBeenCalled();
});

const LOAD_TRIGGER = {
	kind: "special",
	specialEvent: "load",
	manifestRevision: "rev-rendered",
} as const;

/** A hosted app whose device holds the Page contract: prerun on the server, run natively. */
function pageFixture() {
	const { state, backend } = fixture();
	Object.assign(backend, { isLocalOnly: vi.fn(async () => false) });
	mocks.fetcher.mockReset();
	mocks.fetcher.mockImplementation(async (_profile: unknown, path: string) => {
		if (path.endsWith("/prerun")) {
			return {
				board_id: "board",
				runtime_variables: [],
				oauth_requirements: [],
				requires_local_execution: false,
				execution_mode: "Hybrid",
				can_execute_locally: true,
				manifest_revision: "rev-server",
			};
		}
		throw new Error(`unexpected fetch: ${path}`);
	});
	mocks.invoke.mockImplementation(async (command: string) =>
		command === "get_local_page_bootstrap"
			? { executionRevision: "rev-device" }
			: { id: "run" },
	);
	return { state, backend };
}

const prerunCalls = () =>
	mocks.fetcher.mock.calls.filter(([, path]) =>
		String(path).endsWith("/prerun"),
	);

const dispatchLoad = (state: EventState) =>
	state.executeEvent(
		"app",
		"event",
		{ id: "node" },
		false,
		undefined,
		undefined,
		false,
		LOAD_TRIGGER,
	);

/** Every Page test below must end in the native command, never the SSE fallback. */
function expectNativeRuns(count: number) {
	expect(
		mocks.invoke.mock.calls.filter(([command]) => command === "execute_event"),
	).toHaveLength(count);
	expect(mocks.stream).not.toHaveBeenCalled();
}

test("a Page dispatch reuses the governed prerun the execution service just fetched", async () => {
	const { state } = pageFixture();
	await state.prerunEvent("app", "event", undefined, LOAD_TRIGGER);
	await dispatchLoad(state);
	expect(prerunCalls()).toHaveLength(1);
	expectNativeRuns(1);
	expect(mocks.invoke).toHaveBeenCalledWith(
		"execute_event",
		expect.objectContaining({
			pageTrigger: expect.objectContaining({
				kind: "special",
				special_event: "load",
				manifest_revision: "rev-device",
			}),
		}),
	);
});

test("a reused prerun serves one dispatch; the next asks the server again", async () => {
	const { state } = pageFixture();
	await state.prerunEvent("app", "event", undefined, LOAD_TRIGGER);
	await dispatchLoad(state);
	await dispatchLoad(state);
	expect(prerunCalls()).toHaveLength(2);
	expectNativeRuns(2);
});

test("a dispatch's own prerun is never handed to the next dispatch", async () => {
	const { state } = pageFixture();
	await dispatchLoad(state);
	await dispatchLoad(state);
	expect(prerunCalls()).toHaveLength(2);
	expectNativeRuns(2);
});

test("a prerun older than the reuse window is fetched again", async () => {
	vi.useFakeTimers();
	try {
		const { state } = pageFixture();
		await state.prerunEvent("app", "event", undefined, LOAD_TRIGGER);
		vi.advanceTimersByTime(15_001);
		await dispatchLoad(state);
		expect(prerunCalls()).toHaveLength(2);
		expectNativeRuns(1);
	} finally {
		vi.useRealTimers();
	}
});

test("a prerun for another trigger of the same Event is not reused", async () => {
	const { state } = pageFixture();
	await state.prerunEvent("app", "event", undefined, {
		kind: "action",
		actionId: "pa1_click",
		manifestRevision: "rev-rendered",
	});
	await dispatchLoad(state);
	expect(prerunCalls()).toHaveLength(2);
	expectNativeRuns(1);
});

test("a prerun fetched under another account is not reused", async () => {
	const { state, backend } = pageFixture();
	Object.assign(backend, {
		auth: {
			isAuthenticated: true,
			user: { access_token: "token-a", profile: { sub: "user-a" } },
		},
	});
	await state.prerunEvent("app", "event", undefined, LOAD_TRIGGER);
	Object.assign(backend, {
		auth: {
			isAuthenticated: true,
			user: { access_token: "token-b", profile: { sub: "user-b" } },
		},
	});
	await dispatchLoad(state);
	expect(prerunCalls()).toHaveLength(2);
	expectNativeRuns(1);
});
