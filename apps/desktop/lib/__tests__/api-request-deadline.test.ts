import { afterEach, beforeEach, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ fetch: vi.fn() }));

vi.mock("@tauri-apps/plugin-http", () => ({ fetch: mocks.fetch }));
vi.mock("@flow-like/flow-like-ui/lib/api-url", () => ({
	getApiUrl: (_profile: unknown, path: string) => `https://api.test/${path}`,
}));

import { fetcher, fetcherConditional } from "../api";
import { ApiResponseError } from "../api-error";
import {
	DATA_REQUEST_TIMEOUT_MS,
	DISPATCH_REQUEST_TIMEOUT_MS,
	RequestTimeoutError,
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
