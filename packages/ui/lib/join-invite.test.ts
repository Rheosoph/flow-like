import { describe, expect, test } from "bun:test";
import { ApiResponseError } from "./api-error";
import {
	JOIN_MAX_RETRIES,
	attemptJoinWithRetry,
	isTerminalJoinError,
} from "./join-invite";

function apiError(status: number, code?: string): ApiResponseError {
	return new ApiResponseError({ status, code, message: "boom" });
}

describe("isTerminalJoinError", () => {
	test("treats a refusal as a verdict", () => {
		expect(isTerminalJoinError(apiError(404))).toBe(true);
		expect(isTerminalJoinError(apiError(403))).toBe(true);
		expect(isTerminalJoinError(apiError(409, "CONFLICT"))).toBe(true);
	});

	test("keeps transient answers retryable", () => {
		for (const status of [401, 408, 429, 500, 503]) {
			expect(isTerminalJoinError(apiError(status))).toBe(false);
		}
	});

	test("a lost write race is not an exhausted invite link", () => {
		expect(isTerminalJoinError(apiError(409, "DATABASE_CONFLICT"))).toBe(false);
	});

	test("ignores anything that is not an API error", () => {
		expect(isTerminalJoinError(new Error("offline"))).toBe(false);
	});
});

describe("attemptJoinWithRetry", () => {
	test("retries through concurrent-join conflicts and succeeds", async () => {
		let attempts = 0;
		const result = await attemptJoinWithRetry(async () => {
			attempts += 1;
			if (attempts < 3) throw apiError(409, "DATABASE_CONFLICT");
		});
		expect(result).toEqual({ ok: true });
		expect(attempts).toBe(3);
	});

	test("stops on a genuine refusal", async () => {
		let attempts = 0;
		const result = await attemptJoinWithRetry(async () => {
			attempts += 1;
			throw apiError(403);
		});
		expect(attempts).toBe(1);
		expect(result.ok).toBe(false);
		expect(result.kind).toBe("forbidden");
	});
});

test("retry budget is unchanged", () => {
	expect(JOIN_MAX_RETRIES).toBe(6);
});
