import {
	ApiResponseError,
	isTransportFailure,
} from "@flow-like/flow-like-ui/lib/api-error";
import {
	BOARD_FORMAT_HEADER,
	CURRENT_BOARD_FORMAT_VERSION,
} from "@flow-like/flow-like-ui/lib/board-format";
import {
	DATA_REQUEST_TIMEOUT_MS,
	RequestTimeoutError,
	WRITE_REQUEST_TIMEOUT_MS,
} from "@flow-like/flow-like-ui/lib/request-deadline";
import type { AuthContextProps } from "react-oidc-context";
import { afterEach, describe, expect, test, vi } from "vitest";
import { apiFetch, apiPost } from "./api-utils";

const auth = { user: { access_token: "client-token" } } as AuthContextProps;

/** A server that accepted the request and never answers; it only gives up when aborted. */
function stubHangingFetch() {
	const seen: { signal?: AbortSignal } = {};
	const fetchMock = vi.fn(
		(_url: RequestInfo | URL, init?: RequestInit) =>
			new Promise<Response>((_, reject) => {
				seen.signal = init?.signal ?? undefined;
				init?.signal?.addEventListener("abort", () =>
					reject(new DOMException("The operation was aborted.", "AbortError")),
				);
			}),
	);
	vi.stubGlobal("fetch", fetchMock);
	return seen;
}

const outcomeOf = <T>(
	promise: Promise<T>,
): Promise<{ value?: T; error?: unknown }> =>
	promise.then(
		(value) => ({ value }),
		(error: unknown) => ({ error }),
	);

afterEach(() => {
	vi.unstubAllGlobals();
	vi.useRealTimers();
});

test("board requests advertise the client format alongside authentication", async () => {
	let sentHeaders: Headers | undefined;
	vi.stubGlobal(
		"fetch",
		async (_url: RequestInfo | URL, init?: RequestInit) => {
			sentHeaders = new Headers(init?.headers);
			return Response.json({ board_format_version: 2 });
		},
	);
	const result = await apiFetch(
		"apps/example/board/capabilities",
		{ headers: { "x-request-id": "format-negotiation" } },
		auth,
	);
	expect(sentHeaders?.get(BOARD_FORMAT_HEADER)).toBe(
		String(CURRENT_BOARD_FORMAT_VERSION),
	);
	expect(sentHeaders?.get("Authorization")).toBe("Bearer client-token");
	expect(sentHeaders?.get("x-request-id")).toBe("format-negotiation");
	expect(sentHeaders?.has("x-flow-like-capabilities")).toBe(false);
	expect(result).toEqual({ board_format_version: 2 });
});

test.each([
	[
		"a crashed Lambda's error envelope",
		() =>
			Response.json({
				errorType: "Runtime.ExitError",
				errorMessage: "RequestId: r Error: Runtime exited with error",
			}),
	],
	[
		"an HTML page",
		() =>
			new Response("<!doctype html><title>Portal</title>", {
				headers: { "content-type": "text/html" },
			}),
	],
])("%s on 200 is a 502 upstream failure, not data", async (_label, respond) => {
	vi.stubGlobal("fetch", async () => respond());
	const { error } = await outcomeOf(apiFetch("user/groups", undefined, auth));
	expect(error).toBeInstanceOf(ApiResponseError);
	expect((error as ApiResponseError).status).toBe(502);
});

describe("apiFetch deadlines", () => {
	test("a request the server never answers rejects at its route deadline as a transport failure", async () => {
		vi.useFakeTimers();
		const seen = stubHangingFetch();
		const outcome = outcomeOf(
			apiPost("apps/app-1/events/ev-1/prerun", {}, auth),
		);

		await vi.advanceTimersByTimeAsync(WRITE_REQUEST_TIMEOUT_MS - 1);
		expect(seen.signal?.aborted).toBe(false);

		await vi.advanceTimersByTimeAsync(1);
		const { error } = await outcome;
		expect(error).toBeInstanceOf(RequestTimeoutError);
		expect(isTransportFailure(error)).toBe(true);
		expect(seen.signal?.aborted).toBe(true);
		expect((error as Error).message).not.toContain("client-token");
	});

	test("data routes keep the server's longer budget", async () => {
		vi.useFakeTimers();
		const seen = stubHangingFetch();
		const outcome = outcomeOf(
			apiFetch("apps/app-1/db/table/columns", undefined, auth),
		);

		await vi.advanceTimersByTimeAsync(WRITE_REQUEST_TIMEOUT_MS + 1);
		expect(seen.signal?.aborted).toBe(false);

		await vi.advanceTimersByTimeAsync(DATA_REQUEST_TIMEOUT_MS);
		expect((await outcome).error).toBeInstanceOf(RequestTimeoutError);
	});

	test("a caller's abort still cancels the request and is not a timeout", async () => {
		vi.useFakeTimers();
		const seen = stubHangingFetch();
		const controller = new AbortController();
		const outcome = outcomeOf(
			apiFetch("apps/app-1/events/ev-1", { signal: controller.signal }, auth),
		);

		await vi.advanceTimersByTimeAsync(0);
		controller.abort();
		await vi.advanceTimersByTimeAsync(0);
		expect(seen.signal?.aborted).toBe(true);
		const { error } = await outcome;
		expect((error as { name?: string }).name).toBe("AbortError");
		expect(vi.getTimerCount()).toBe(0);
	});

	test("a response in time is returned and disarms the deadline", async () => {
		vi.useFakeTimers();
		vi.stubGlobal("fetch", async () => Response.json({ ok: true }));
		await expect(
			apiFetch("apps/app-1/events/ev-1", undefined, auth),
		).resolves.toEqual({ ok: true });
		expect(vi.getTimerCount()).toBe(0);
	});
});
