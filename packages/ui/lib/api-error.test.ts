import { describe, expect, test } from "bun:test";
import {
	ApiResponseError,
	UPSTREAM_UNAVAILABLE_CODE,
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
