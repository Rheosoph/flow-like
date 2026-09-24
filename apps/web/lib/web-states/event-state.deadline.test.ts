import { isTransportFailure } from "@flow-like/flow-like-ui/lib/api-error";
import {
	DEFAULT_REQUEST_TIMEOUT_MS,
	DISPATCH_REQUEST_TIMEOUT_MS,
	RequestTimeoutError,
	STREAM_HEADER_TIMEOUT_MS,
} from "@flow-like/flow-like-ui/lib/request-deadline";
import {
	resetRunTiming,
	startRunTrace,
} from "@flow-like/flow-like-ui/lib/run-timing";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	apiGet: vi.fn(),
	apiDelete: vi.fn(),
	cancelDeviceCommands: vi.fn(),
	checkOAuthTokens: vi.fn(),
	checkOAuthTokensFromPrerun: vi.fn(),
	getConsentedProviderIds: vi.fn(),
}));

vi.mock("@flow-like/flow-like-ui", () => ({
	checkOAuthTokens: mocks.checkOAuthTokens,
	checkOAuthTokensFromPrerun: mocks.checkOAuthTokensFromPrerun,
	classifyPageContractError: () => undefined,
	finishAllProgressToasts: vi.fn(),
	getCurrentPageContext: () => undefined,
	notifyPageContractRejected: vi.fn(),
	serializePageTrigger: (trigger: unknown) => trigger,
	showProgressToast: vi.fn(),
	withCurrentManifestRevision: (trigger: unknown) => trigger,
}));
vi.mock("@flow-like/flow-like-ui/components/payments/payment-events", () => ({
	dispatchPaymentRequest: vi.fn(),
}));
vi.mock("@flow-like/flow-like-ui/lib/device-bridge", () => ({
	cancelDeviceCommands: mocks.cancelDeviceCommands,
	withDeviceCommandBridge: (
		_context: unknown,
		callback: unknown,
		run: (callback: unknown) => Promise<unknown>,
	) => run(callback),
}));
vi.mock("./api-utils", () => ({
	apiGet: mocks.apiGet,
	apiDelete: mocks.apiDelete,
	getApiBaseUrl: () => "https://api.test",
}));
vi.mock("../oauth-db", () => ({
	oauthConsentStore: {
		getConsentedProviderIds: mocks.getConsentedProviderIds,
	},
	oauthTokenStore: {},
}));
vi.mock("../oauth-service", () => ({
	getOAuthApiBaseUrl: () => "https://api.test",
	getOAuthService: () => ({ refreshToken: vi.fn() }),
}));
vi.mock("sonner", () => ({ toast: vi.fn() }));

import { WebEventState } from "./event-state";

const auth = { user: { access_token: "token-a" } } as never;
const noProviders = { requiredProviders: [], missingProviders: [], tokens: {} };

/** A server that accepted the request and never answers; it only gives up when aborted. */
function hangingResponse(init?: RequestInit): Promise<Response> {
	return new Promise((_, reject) => {
		init?.signal?.addEventListener("abort", () =>
			reject(new DOMException("The operation was aborted.", "AbortError")),
		);
	});
}

const outcomeOf = <T>(
	promise: Promise<T>,
): Promise<{ value?: T; error?: unknown }> =>
	promise.then(
		(value) => ({ value }),
		(error: unknown) => ({ error }),
	);

function sseFrame(event: Record<string, unknown>): Uint8Array {
	return new TextEncoder().encode(`data: ${JSON.stringify(event)}\n\n`);
}

function execute(state: WebEventState, cb = vi.fn(), onEventId = vi.fn()) {
	return state.executeEvent(
		"app-1",
		"event-1",
		{ id: "node-1", payload: {} } as never,
		false,
		onEventId,
		cb,
		true,
	);
}

beforeEach(() => {
	vi.useFakeTimers();
	vi.spyOn(console, "log").mockImplementation(() => {});
	vi.spyOn(console, "info").mockImplementation(() => {});
	vi.spyOn(console, "warn").mockImplementation(() => {});
	mocks.apiGet.mockImplementation(async (path: string) =>
		path.includes("/board/") ? {} : { board_id: "board-1" },
	);
	mocks.checkOAuthTokens.mockResolvedValue(noProviders);
	mocks.checkOAuthTokensFromPrerun.mockResolvedValue(noProviders);
	mocks.getConsentedProviderIds.mockResolvedValue(new Set());
});

afterEach(() => {
	vi.unstubAllGlobals();
	vi.restoreAllMocks();
	vi.useRealTimers();
	resetRunTiming();
});

const SIXTEEN_MINUTES_MS = 16 * 60_000;

/** An SSE body the test feeds by hand; `cancelled` records a reader cancelling it. */
function manualSseResponse() {
	const seen: {
		signal?: AbortSignal;
		stream?: ReadableStreamDefaultController<Uint8Array>;
		cancelled: boolean;
	} = { cancelled: false };
	vi.stubGlobal(
		"fetch",
		vi.fn(async (_url: string, init?: RequestInit) => {
			seen.signal = init?.signal ?? undefined;
			return new Response(
				new ReadableStream<Uint8Array>({
					start(controller) {
						seen.stream = controller;
					},
					cancel() {
						seen.cancelled = true;
					},
				}),
				{ status: 200, headers: { "content-type": "text/event-stream" } },
			);
		}),
	);
	return seen;
}

describe("web invoke deadline", () => {
	test("an invoke whose headers never arrive rejects at the dispatch deadline as a transport failure", async () => {
		let signal: AbortSignal | undefined;
		vi.stubGlobal(
			"fetch",
			vi.fn((_url: string, init?: RequestInit) => {
				signal = init?.signal ?? undefined;
				return hangingResponse(init);
			}),
		);
		const outcome = outcomeOf(execute(new WebEventState({ auth })));

		// A cold dispatch (compile, admission, executor start) outlasts a plain stream deadline.
		await vi.advanceTimersByTimeAsync(STREAM_HEADER_TIMEOUT_MS);
		expect(signal?.aborted).toBe(false);

		await vi.advanceTimersByTimeAsync(
			DISPATCH_REQUEST_TIMEOUT_MS - STREAM_HEADER_TIMEOUT_MS - 1,
		);
		expect(signal?.aborted).toBe(false);

		await vi.advanceTimersByTimeAsync(1);
		const { error } = await outcome;
		expect(error).toBeInstanceOf(RequestTimeoutError);
		expect(isTransportFailure(error)).toBe(true);
		expect(signal?.aborted).toBe(true);
	});

	test("a dispatch that fails before the run id is timed as a dispatch failure", async () => {
		vi.stubGlobal(
			"fetch",
			vi.fn(async () => {
				throw new TypeError("Failed to fetch");
			}),
		);
		const trace = startRunTrace("invoke-dispatch-error");

		expect(
			(await outcomeOf(execute(new WebEventState({ auth })))).error,
		).toBeInstanceOf(TypeError);
		const steps = trace.finish()?.steps.map((step) => step.name);
		expect(steps).toContain("invoke.until_error");
		expect(steps).not.toContain("invoke.until_run_initiated");
	});

	test("a 16-minute SSE body keeps streaming once headers arrived", async () => {
		const seen = manualSseResponse();
		const cb = vi.fn();
		const onEventId = vi.fn();
		const trace = startRunTrace("invoke-stream");
		const outcome = outcomeOf(
			execute(new WebEventState({ auth }), cb, onEventId),
		);

		await vi.advanceTimersByTimeAsync(0);
		seen.stream?.enqueue(
			sseFrame({ event_type: "run_initiated", payload: { run_id: "run-1" } }),
		);
		await vi.advanceTimersByTimeAsync(SIXTEEN_MINUTES_MS);
		expect(seen.signal?.aborted).toBe(false);
		expect(onEventId).toHaveBeenCalledWith("run-1");

		seen.stream?.enqueue(
			sseFrame({ event_type: "chat_out", payload: { delta: "late" } }),
		);
		await vi.advanceTimersByTimeAsync(SIXTEEN_MINUTES_MS);
		expect(cb).toHaveBeenCalledTimes(2);
		expect(seen.signal?.aborted).toBe(false);

		seen.stream?.enqueue(sseFrame({ event_type: "completed", payload: {} }));
		await vi.advanceTimersByTimeAsync(0);
		const { error } = await outcome;
		expect(error).toBeUndefined();
		expect(cb).toHaveBeenCalledTimes(3);
		expect(seen.signal?.aborted).toBe(false);
		expect(vi.getTimerCount()).toBe(0);
		const steps = trace.finish()?.steps.map((step) => step.name);
		expect(steps).toContain("invoke.until_run_initiated");
		expect(steps).not.toContain("invoke.until_error");
	});

	test("a terminal event releases the reader while the server still holds the body open", async () => {
		const seen = manualSseResponse();
		const cb = vi.fn();
		const outcome = outcomeOf(execute(new WebEventState({ auth }), cb));

		await vi.advanceTimersByTimeAsync(0);
		seen.stream?.enqueue(
			sseFrame({ event_type: "run_initiated", payload: { run_id: "run-1" } }),
		);
		seen.stream?.enqueue(sseFrame({ event_type: "completed", payload: {} }));
		await vi.advanceTimersByTimeAsync(0);

		expect((await outcome).error).toBeUndefined();
		expect(seen.cancelled).toBe(true);
		expect(cb).toHaveBeenCalledTimes(2);
	});

	test("a failure after the run id arrived is not reported as a dispatch failure", async () => {
		let stream: ReadableStreamDefaultController<Uint8Array> | undefined;
		vi.stubGlobal(
			"fetch",
			vi.fn(
				async () =>
					new Response(
						new ReadableStream<Uint8Array>({
							start(controller) {
								stream = controller;
							},
						}),
					),
			),
		);
		const trace = startRunTrace("invoke-late-error");
		const outcome = outcomeOf(execute(new WebEventState({ auth })));

		await vi.advanceTimersByTimeAsync(0);
		stream?.enqueue(
			sseFrame({ event_type: "run_initiated", payload: { run_id: "run-1" } }),
		);
		await vi.advanceTimersByTimeAsync(0);
		stream?.error(new TypeError("network error"));
		await vi.advanceTimersByTimeAsync(0);

		expect((await outcome).error).toBeInstanceOf(TypeError);
		const steps = trace.finish()?.steps.map((step) => step.name);
		expect(steps).toContain("invoke.until_run_initiated");
		expect(steps).not.toContain("invoke.until_error");
	});
});

describe("web run cancellation", () => {
	test("cancelling a running invoke stops the server run and closes this client's stream", async () => {
		mocks.apiDelete.mockResolvedValue({ cancelled: true });
		let signal: AbortSignal | undefined;
		let stream: ReadableStreamDefaultController<Uint8Array> | undefined;
		vi.stubGlobal(
			"fetch",
			vi.fn(async (_url: string, init?: RequestInit) => {
				signal = init?.signal ?? undefined;
				return new Response(
					new ReadableStream<Uint8Array>({
						start(controller) {
							stream = controller;
							// Like a real fetch, aborting its signal errors a body still being read.
							signal?.addEventListener("abort", () =>
								controller.error(
									new DOMException("The operation was aborted.", "AbortError"),
								),
							);
						},
					}),
				);
			}),
		);
		const state = new WebEventState({ auth });
		const cb = vi.fn();
		const outcome = outcomeOf(execute(state, cb));

		await vi.advanceTimersByTimeAsync(0);
		stream?.enqueue(
			sseFrame({ event_type: "run_initiated", payload: { run_id: "run/1" } }),
		);
		await vi.advanceTimersByTimeAsync(0);
		await state.cancelExecution("run/1");

		expect(mocks.cancelDeviceCommands).toHaveBeenCalledWith("run/1");
		expect(mocks.apiDelete).toHaveBeenCalledWith("execution/run/run%2F1", auth);
		expect(signal?.aborted).toBe(true);
		expect((await outcome).error).toBeDefined();
		expect(cb).toHaveBeenCalledTimes(1);
	});

	test("a run that already ended is only cancelled on the server", async () => {
		mocks.apiDelete.mockResolvedValue({ cancelled: false });
		vi.stubGlobal(
			"fetch",
			vi.fn(
				async () =>
					new Response(
						new ReadableStream<Uint8Array>({
							start(controller) {
								controller.enqueue(
									sseFrame({
										event_type: "run_initiated",
										payload: { run_id: "run-2" },
									}),
								);
								controller.enqueue(
									sseFrame({ event_type: "completed", payload: {} }),
								);
								controller.close();
							},
						}),
					),
			),
		);
		const state = new WebEventState({ auth });
		await execute(state);

		await state.cancelExecution("run-2");
		expect(mocks.apiDelete).toHaveBeenCalledWith("execution/run/run-2", auth);
	});
});

describe("hub config", () => {
	test("a failed hub fetch is retried on the next call, and only a success is cached", async () => {
		const hub = { oauth_providers: { github: { id: "github" } } };
		const fetchMock = vi
			.fn()
			.mockRejectedValueOnce(new TypeError("Failed to fetch"))
			.mockResolvedValueOnce(
				Response.json({ error: "unavailable" }, { status: 503 }),
			)
			.mockImplementationOnce((_url: string, init?: RequestInit) =>
				hangingResponse(init),
			)
			.mockResolvedValue(Response.json(hub));
		vi.stubGlobal("fetch", fetchMock);
		const state = new WebEventState({
			auth,
			profile: { hub: "hub.test" } as never,
		});
		const hubPassedToOAuth = () =>
			mocks.checkOAuthTokensFromPrerun.mock.calls.at(-1)?.[2];

		await state.checkOAuthRequirements("app-1", []);
		expect(hubPassedToOAuth()).toBeUndefined();

		await state.checkOAuthRequirements("app-1", []);
		expect(hubPassedToOAuth()).toBeUndefined();

		const stalled = state.checkOAuthRequirements("app-1", []);
		await vi.advanceTimersByTimeAsync(DEFAULT_REQUEST_TIMEOUT_MS);
		await stalled;
		expect(hubPassedToOAuth()).toBeUndefined();

		await state.checkOAuthRequirements("app-1", []);
		expect(hubPassedToOAuth()).toEqual(hub);

		await state.checkOAuthRequirements("app-1", []);
		expect(hubPassedToOAuth()).toEqual(hub);
		expect(fetchMock).toHaveBeenCalledTimes(4);
		expect(fetchMock).toHaveBeenCalledWith(
			"https://hub.test/api/v1",
			expect.objectContaining({ signal: expect.any(AbortSignal) }),
		);
	});
});
