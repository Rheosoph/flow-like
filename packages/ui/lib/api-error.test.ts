import { describe, expect, test } from "bun:test";
import {
	ApiResponseError,
	UPSTREAM_UNAVAILABLE_CODE,
	apiResponseError,
	isMissingResourceError,
	isTransportFailure,
	upstreamFailureInSuccess,
} from "./api-error";

class RequestTimeoutError extends Error {
	constructor() {
		super("Request timed out after 45000ms: POST apps/a/events/e/prerun");
		this.name = "RequestTimeoutError";
	}
}

describe("isTransportFailure", () => {
	test.each([
		["a desktop request deadline", new RequestTimeoutError()],
		["the desktop offline guard", new Error("Network unavailable: GET apps/a")],
		["a browser fetch failure", new TypeError("Failed to fetch")],
		["a Safari fetch failure", new TypeError("Load failed")],
		["a Node fetch failure", new TypeError("fetch failed")],
	])("%s is inconclusive", (_label, error) => {
		expect(isTransportFailure(error)).toBe(true);
	});

	test.each([
		[
			"a server refusal",
			new ApiResponseError({
				status: 403,
				message: "Forbidden",
				path: "apps/a",
			}),
		],
		[
			"a server error",
			new ApiResponseError({ status: 500, message: "boom", path: "apps/a" }),
		],
		["a missing resource", new Error("Event not found: e")],
		["nothing", undefined],
		["a bare string", "Forbidden"],
	])("%s is a real answer", (_label, error) => {
		expect(isTransportFailure(error)).toBe(false);
	});
});

describe("isMissingResourceError", () => {
	test("the API's own 404 and 410 say the resource is gone", () => {
		for (const status of [404, 410]) {
			expect(
				isMissingResourceError(
					new ApiResponseError({ status, code: "NOT_FOUND", message: "x" }),
				),
			).toBe(true);
		}
	});

	test("a bare 404 from a proxy or a hub without the route is inconclusive", () => {
		expect(
			isMissingResourceError(
				new ApiResponseError({ status: 404, message: "Not Found" }),
			),
		).toBe(false);
		expect(isMissingResourceError({ status: 404 })).toBe(false);
	});
});

describe("apiResponseError retryAfter", () => {
	const tooMany = (headers: Record<string, string>, body: unknown) =>
		apiResponseError(
			{
				status: 429,
				statusText: "Too Many Requests",
				headers: new Headers(headers),
			},
			JSON.stringify(body),
			"devices/d/certificate-notices/test",
		);
	const refusal = {
		error: { code: "TOO_MANY_REQUESTS", message: "Try again in 540 seconds." },
	};

	test("the Retry-After header wins over the body", () => {
		const error = tooMany(
			{ "Retry-After": "540" },
			{ ...refusal, retry_after: 600 },
		);
		expect(error.retryAfter).toBe(540);
		expect(error.code).toBe("TOO_MANY_REQUESTS");
		expect(error.toJSON().retryAfter).toBe(540);
	});

	test("the body's top-level retry_after is used when the header is hidden", () => {
		expect(tooMany({}, { ...refusal, retry_after: 540 }).retryAfter).toBe(540);
	});

	test.each([
		["no wait at all", {}, refusal],
		[
			"an HTTP date",
			{ "Retry-After": "Fri, 02 Oct 2026 15:00:00 GMT" },
			refusal,
		],
		["a negative body value", {}, { ...refusal, retry_after: -1 }],
		["a non-numeric body value", {}, { ...refusal, retry_after: "soon" }],
	])("%s leaves it undefined", (_label, headers, body) => {
		expect(tooMany(headers, body).retryAfter).toBeUndefined();
	});

	test("a non-JSON body keeps the header", () => {
		const error = apiResponseError(
			{
				status: 429,
				statusText: "",
				headers: new Headers({ "retry-after": "7" }),
			},
			"slow down",
		);
		expect(error.retryAfter).toBe(7);
		expect(error.serverMessage).toBe("slow down");
	});
});

describe("upstreamFailureInSuccess", () => {
	const ok = (contentType = "application/json", requestId?: string) => ({
		status: 200,
		headers: new Headers({
			"content-type": contentType,
			...(requestId ? { "x-amzn-requestid": requestId } : {}),
		}),
	});

	test("a Lambda runtime envelope on 200 is a 502 upstream failure", () => {
		const error = upstreamFailureInSuccess(
			ok("application/json", "req-1"),
			{
				errorType: "Runtime.ExitError",
				errorMessage: "RequestId: req-1 Error: Runtime exited with error",
			},
			"user/groups",
		);
		expect(error).toBeInstanceOf(ApiResponseError);
		expect(error?.status).toBe(502);
		expect(error?.code).toBe(UPSTREAM_UNAVAILABLE_CODE);
		expect(error?.errorId).toBe("req-1");
		expect(error?.path).toBe("user/groups");
	});

	test("an HTML page on 200 is a 502 upstream failure", () => {
		const error = upstreamFailureInSuccess(
			ok("text/html; charset=utf-8"),
			"<!doctype html><title>Sign in to Wi-Fi</title>",
		);
		expect(error?.status).toBe(502);
		expect(error?.code).toBe(UPSTREAM_UNAVAILABLE_CODE);
	});

	test.each([
		["an array", [{ id: "a" }]],
		["a record", { id: "a", errorMessage: "user text" }],
		["plain text", "ok"],
		["nothing", undefined],
	])("%s is real data", (_label, data) => {
		expect(upstreamFailureInSuccess(ok(), data)).toBeUndefined();
	});
});
