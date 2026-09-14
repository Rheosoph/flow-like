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
