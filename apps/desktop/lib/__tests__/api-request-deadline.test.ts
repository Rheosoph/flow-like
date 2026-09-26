import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ fetch: vi.fn() }));

vi.mock("@tauri-apps/plugin-http", () => ({ fetch: mocks.fetch }));
vi.mock("@flow-like/flow-like-ui/lib/api-url", () => ({
	getApiUrl: (_profile: unknown, path: string) => `https://api.test/${path}`,
}));

import { isTransportFailure } from "@flow-like/flow-like-ui/lib/api-error";
import { fetcher, fetcherConditional, streamFetcher } from "../api";
import { ApiResponseError } from "../api-error";
import {
	DATA_REQUEST_TIMEOUT_MS,
	DISPATCH_REQUEST_TIMEOUT_MS,
	RequestTimeoutError,
	STREAM_HEADER_TIMEOUT_MS,
	UPLOAD_FLOOR_BYTES_PER_SECOND,
	WRITE_REQUEST_TIMEOUT_MS,
} from "../request-deadline";

const profile = { id: "profile-1", hub: "hub.test" } as never;
const auth = { user: { access_token: "token-a" } } as never;

/**
 * A socket that accepted the request and never answers. Like plugin-http, the
 * request only rejects once its cancel IPC has round-tripped, never synchronously.
 */
function hangingFetch() {
	const seen: { signal?: AbortSignal } = {};
	mocks.fetch.mockImplementation(
		(_url: string, init: { signal?: AbortSignal }) => {
			seen.signal = init.signal;
			return new Promise((_, reject) => {
				init.signal?.addEventListener("abort", () =>
					setTimeout(
						() =>
							reject(
								new DOMException("The operation was aborted.", "AbortError"),
							),
						0,
					),
				);
			});
		},
	);
	return seen;
}

function jsonResponse(status: number, body: unknown, headers?: HeadersInit) {
	return {
		ok: status >= 200 && status < 300,
		status,
		statusText: String(status),
		headers: new Headers(headers),
		text: async () => (body === undefined ? "" : JSON.stringify(body)),
	};
}

/** Settles into either the value or the rejection, so timers can be advanced first. */
const outcomeOf = <T>(
	promise: Promise<T>,
): Promise<{ value?: T; error?: unknown }> =>
	promise.then(
		(value) => ({ value }),
		(error: unknown) => ({ error }),
	);

beforeEach(() => {
	vi.useFakeTimers();
	vi.stubGlobal("navigator", { onLine: true });
	mocks.fetch.mockReset();
});

afterEach(() => {
	vi.unstubAllGlobals();
	vi.useRealTimers();
});

test("a request whose socket never answers settles at the route deadline and aborts natively", async () => {
	const seen = hangingFetch();
	const outcome = outcomeOf(
		fetcher(
			profile,
			"apps/app-1/events/ev-1/prerun",
			{ method: "POST", body: "{}" },
			auth,
		),
	);

	await vi.advanceTimersByTimeAsync(WRITE_REQUEST_TIMEOUT_MS - 1);
	expect(seen.signal?.aborted).toBe(false);

	await vi.advanceTimersByTimeAsync(1);
	expect((await outcome).error).toBeInstanceOf(RequestTimeoutError);
	expect(seen.signal?.aborted).toBe(true);
});

test("data routes keep the server's longer budget", async () => {
	const seen = hangingFetch();
	const outcome = outcomeOf(
		fetcher(profile, "apps/app-1/db/table/columns", undefined, auth),
	);

	await vi.advanceTimersByTimeAsync(WRITE_REQUEST_TIMEOUT_MS + 1);
	expect(seen.signal?.aborted).toBe(false);

	await vi.advanceTimersByTimeAsync(DATA_REQUEST_TIMEOUT_MS);
	expect((await outcome).error).toBeInstanceOf(RequestTimeoutError);
});

test("an Event upsert waits for the executor setup the server runs synchronously", async () => {
	const seen = hangingFetch();
	const outcome = outcomeOf(
		fetcher(
			profile,
			"apps/app-1/events/ev-1",
			{ method: "PUT", body: JSON.stringify({ id: "ev-1" }) },
			auth,
		),
	);

	await vi.advanceTimersByTimeAsync(DATA_REQUEST_TIMEOUT_MS);
	expect(seen.signal?.aborted).toBe(false);

	await vi.advanceTimersByTimeAsync(DISPATCH_REQUEST_TIMEOUT_MS);
	expect((await outcome).error).toBeInstanceOf(RequestTimeoutError);
});

test("a large body extends the deadline by its upload time", async () => {
	const seen = hangingFetch();
	const body = "x".repeat(4 * 1024 * 1024);
	const allowance =
		Math.floor(body.length / UPLOAD_FLOOR_BYTES_PER_SECOND) * 1000;
	const outcome = outcomeOf(
		fetcher(
			profile,
			"apps/app-1/board/board-1",
			{ method: "POST", body },
			auth,
		),
	);

	await vi.advanceTimersByTimeAsync(DATA_REQUEST_TIMEOUT_MS + allowance - 1);
	expect(seen.signal?.aborted).toBe(false);

	await vi.advanceTimersByTimeAsync(1);
	expect((await outcome).error).toBeInstanceOf(RequestTimeoutError);
});

test("a caller's own deadline overrides the route class", async () => {
	const seen = hangingFetch();
	const outcome = outcomeOf(
		fetcher(
			profile,
			"apps/app-1/events/ev-1/prerun",
			{ method: "POST", timeoutMs: 1_000 },
			auth,
		),
	);

	await vi.advanceTimersByTimeAsync(1_000);
	expect((await outcome).error).toBeInstanceOf(RequestTimeoutError);
	expect(seen.signal?.aborted).toBe(true);
	// The override is consumed here, never forwarded to the native request.
	expect(mocks.fetch.mock.calls[0]?.[1]).not.toHaveProperty("timeoutMs");
});

test("a caller's abort signal still cancels the native request", async () => {
	const seen = hangingFetch();
	const controller = new AbortController();
	const outcome = outcomeOf(
		fetcher(
			profile,
			"apps/app-1/events/ev-1/prerun",
			{ method: "POST", signal: controller.signal },
			auth,
		),
	);

	controller.abort();
	await vi.advanceTimersByTimeAsync(0);
	expect(seen.signal?.aborted).toBe(true);
	// The caller's abort is not a deadline: its own error surfaces and the timer is torn down.
	const { error } = await outcome;
	expect((error as { name?: string }).name).toBe("AbortError");
	expect(vi.getTimerCount()).toBe(0);
});

test("a response that arrives in time is returned and disarms the deadline", async () => {
	mocks.fetch.mockResolvedValue(jsonResponse(200, { board_id: "board-1" }));

	await expect(
		fetcher(profile, "apps/app-1/events/ev-1/prerun", { method: "POST" }, auth),
	).resolves.toEqual({ board_id: "board-1" });
	expect(vi.getTimerCount()).toBe(0);
});

test("a 304 against the offered tag is reported as not modified", async () => {
	mocks.fetch.mockResolvedValue(
		jsonResponse(304, undefined, { etag: 'W/"b"' }),
	);

	await expect(
		fetcherConditional(
			profile,
			"apps/app-1/pages/page-1",
			{ method: "GET" },
			auth,
			'W/"a"',
		),
	).resolves.toEqual({ notModified: true, etag: 'W/"b"' });
});

test("a server refusal is surfaced as the API error, not rewrapped", async () => {
	mocks.fetch.mockResolvedValue(jsonResponse(500, { message: "boom" }));

	const { error } = await outcomeOf(
		fetcher(profile, "apps/app-1/events/ev-1/prerun", { method: "POST" }, auth),
	);
	expect(error).toBeInstanceOf(ApiResponseError);
	expect(vi.getTimerCount()).toBe(0);
});

test("a crashed API Lambda answering 200 with its error envelope is a 502, not data", async () => {
	mocks.fetch.mockResolvedValue(
		jsonResponse(200, {
			errorType: "Runtime.ExitError",
			errorMessage: "RequestId: r Error: Runtime exited with error",
		}),
	);

	const { error } = await outcomeOf(
		fetcher(profile, "user/groups", undefined, auth),
	);
	expect(error).toBeInstanceOf(ApiResponseError);
	expect((error as ApiResponseError).status).toBe(502);
	expect(vi.getTimerCount()).toBe(0);
});

test("a connection plugin-http rejects with bare text is reported as a transport failure", async () => {
	const rejection =
		"error sending request for url (https://api.test/apps/app-1/pages/bootstrap)";
	mocks.fetch.mockRejectedValue(rejection);

	const { error } = await outcomeOf(
		fetcher(profile, "apps/app-1/pages/bootstrap", undefined, auth),
	);
	expect(isTransportFailure(error)).toBe(true);
	expect((error as Error).cause).toBe(rejection);
});

const SIXTEEN_MINUTES_MS = 16 * 60_000;

/** An SSE body the test feeds by hand; like plugin-http, aborting the signal errors it. */
function manualStream() {
	const seen: {
		signal?: AbortSignal;
		body?: ReadableStreamDefaultController<Uint8Array>;
	} = {};
	mocks.fetch.mockImplementation(
		async (_url: string, init: { signal?: AbortSignal }) => {
			seen.signal = init.signal;
			return new Response(
				new ReadableStream<Uint8Array>({
					start(controller) {
						seen.body = controller;
						init.signal?.addEventListener("abort", () =>
							controller.error("Request cancelled"),
						);
					},
				}),
				{ status: 200, headers: { "content-type": "text/event-stream" } },
			);
		},
	);
	return seen;
}

const sseFrame = (event: Record<string, unknown>) =>
	new TextEncoder().encode(`data: ${JSON.stringify(event)}\n\n`);

function stream(path: string, init?: RequestInit, onMessage = vi.fn()) {
	return streamFetcher(
		profile,
		path,
		{ method: "POST", body: "{}", ...init },
		auth,
		onMessage,
	);
}

describe("POST streams", () => {
	beforeEach(() => {
		vi.spyOn(console, "log").mockImplementation(() => {});
	});

	afterEach(() => {
		vi.restoreAllMocks();
	});

	test.each([
		"apps/app-1/events/ev-1/invoke",
		"apps/app-1/board/board-1/invoke",
	])("%s waits for its headers as long as its dispatch", async (path) => {
		const seen = hangingFetch();
		const outcome = outcomeOf(stream(path));

		await vi.advanceTimersByTimeAsync(STREAM_HEADER_TIMEOUT_MS);
		expect(seen.signal?.aborted).toBe(false);

		await vi.advanceTimersByTimeAsync(
			DISPATCH_REQUEST_TIMEOUT_MS - STREAM_HEADER_TIMEOUT_MS - 1,
		);
		expect(seen.signal?.aborted).toBe(false);

		await vi.advanceTimersByTimeAsync(1);
		expect((await outcome).error).toBeInstanceOf(RequestTimeoutError);
		expect(seen.signal?.aborted).toBe(true);
	});

	test("a large invoke body extends the header deadline by its upload time", async () => {
		const seen = hangingFetch();
		const body = "x".repeat(4 * 1024 * 1024);
		const allowance =
			Math.floor(body.length / UPLOAD_FLOOR_BYTES_PER_SECOND) * 1000;
		const outcome = outcomeOf(
			stream("apps/app-1/events/ev-1/invoke", { body }),
		);

		await vi.advanceTimersByTimeAsync(
			DISPATCH_REQUEST_TIMEOUT_MS + allowance - 1,
		);
		expect(seen.signal?.aborted).toBe(false);

		await vi.advanceTimersByTimeAsync(1);
		expect((await outcome).error).toBeInstanceOf(RequestTimeoutError);
	});

	test("a stream on any other route keeps the plain header deadline", async () => {
		const seen = hangingFetch();
		const outcome = outcomeOf(stream("apps/app-1/events/ev-1/prerun"));

		await vi.advanceTimersByTimeAsync(STREAM_HEADER_TIMEOUT_MS - 1);
		expect(seen.signal?.aborted).toBe(false);

		await vi.advanceTimersByTimeAsync(1);
		expect((await outcome).error).toBeInstanceOf(RequestTimeoutError);
		expect(seen.signal?.aborted).toBe(true);
	});

	test("a 16-minute body keeps streaming once headers arrived", async () => {
		const seen = manualStream();
		const onMessage = vi.fn();
		const outcome = outcomeOf(
			stream("apps/app-1/events/ev-1/invoke", undefined, onMessage),
		);

		await vi.advanceTimersByTimeAsync(0);
		seen.body?.enqueue(sseFrame({ event_type: "run_initiated" }));
		await vi.advanceTimersByTimeAsync(SIXTEEN_MINUTES_MS);
		seen.body?.enqueue(sseFrame({ event_type: "chat_out" }));
		await vi.advanceTimersByTimeAsync(SIXTEEN_MINUTES_MS);
		expect(seen.signal?.aborted).toBe(false);
		expect(onMessage).toHaveBeenCalledTimes(2);

		seen.body?.enqueue(sseFrame({ event_type: "completed" }));
		expect((await outcome).error).toBeUndefined();
		expect(onMessage).toHaveBeenCalledTimes(3);
		// The terminal event closes the connection.
		expect(seen.signal?.aborted).toBe(true);
		expect(vi.getTimerCount()).toBe(0);
	});

	test("the caller's signal aborts the stream after its headers arrived", async () => {
		const seen = manualStream();
		const caller = new AbortController();
		const onMessage = vi.fn();
		const outcome = outcomeOf(
			stream(
				"apps/app-1/events/ev-1/invoke",
				{ signal: caller.signal },
				onMessage,
			),
		);

		await vi.advanceTimersByTimeAsync(0);
		seen.body?.enqueue(sseFrame({ event_type: "run_initiated" }));
		await vi.advanceTimersByTimeAsync(0);
		expect(onMessage).toHaveBeenCalledTimes(1);

		caller.abort();
		expect(seen.signal?.aborted).toBe(true);
		expect((await outcome).error).toBe("Request cancelled");
	});

	test("a caller that aborted before sending never opens the stream", async () => {
		// Like plugin-http, an already aborted signal fails before any IPC.
		mocks.fetch.mockImplementation(
			async (_url: string, init: { signal?: AbortSignal }) => {
				if (init.signal?.aborted) throw new Error("Request cancelled");
				throw new Error("sent despite the abort");
			},
		);
		const caller = new AbortController();
		caller.abort();

		const { error } = await outcomeOf(
			stream("apps/app-1/events/ev-1/invoke", { signal: caller.signal }),
		);
		expect((error as Error).message).toBe("Request cancelled");
		expect(vi.getTimerCount()).toBe(0);
	});
});

test("any other bare plugin-http rejection keeps its text", async () => {
	mocks.fetch.mockRejectedValue(
		"url not allowed on the configured scope: https://api.test/apps",
	);

	const { error } = await outcomeOf(
		fetcher(profile, "apps/app-1/pages/bootstrap", undefined, auth),
	);
	expect(isTransportFailure(error)).toBe(false);
	expect((error as Error).message).toContain("url not allowed");
});
