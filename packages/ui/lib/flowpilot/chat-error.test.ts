import { describe, expect, test } from "vitest";

import { type ApiResponseError, apiResponseError } from "../api-error";
import { buildChatMessageError } from "./chat-error";

function apiFailure(status: number, body: unknown): ApiResponseError {
	return apiResponseError(
		{
			status,
			statusText: "",
			headers: new Headers(),
		},
		JSON.stringify(body),
		"/api/v1/ai/global-chat",
	);
}

describe("buildChatMessageError", () => {
	test("routes a profile without models to the model settings", () => {
		const error = buildChatMessageError(
			"bits",
			apiFailure(400, {
				error: {
					code: "BAD_REQUEST",
					message:
						"This profile has no language model. Add one in Settings → Models before using FlowPilot.",
				},
			}),
		);

		expect(error.kind).toBe("config");
		expect(error.title).toBe("No model configured");
		expect(error.message).toContain("Add one in Settings");
		expect(error.action).toEqual({
			kind: "navigate",
			label: "Open model settings",
			href: "/settings/ai?tab=models",
		});
		expect(error.code).toBe("BAD_REQUEST");
		expect(error.status).toBe(400);
	});

	test("routes a plan rejection into the upgrade dialog", () => {
		const error = buildChatMessageError(
			"bits",
			apiFailure(402, {
				error: {
					code: "PAYMENT_REQUIRED",
					message:
						"None of the models in this profile are included in your plan.",
				},
			}),
		);

		expect(error.kind).toBe("billing");
		expect(error.action?.kind).toBe("upgrade");
		expect(error.retryable).toBe(false);
	});

	test("keeps the incident reference of a server failure", () => {
		const error = buildChatMessageError(
			"bits",
			apiFailure(500, {
				error: { code: "INTERNAL_ERROR", id: "err_123", message: "Error" },
			}),
		);

		expect(error.kind).toBe("server");
		expect(error.reference).toBe("err_123");
		expect(error.retryable).toBe(true);
	});

	test("recovers the envelope from a stringified transport failure", () => {
		const error = buildChatMessageError(
			"bits",
			new Error(
				'FlowPilot request failed (429): {"error":{"code":"TOO_MANY_REQUESTS","message":"Slow down."}}',
			),
		);

		expect(error.kind).toBe("rate-limit");
		expect(error.message).toBe("Slow down.");
		expect(error.status).toBe(429);
	});

	test("reads a stopped run as a stop, not a failure", () => {
		const error = buildChatMessageError("bits", {
			name: "AbortError",
			message: "The user aborted a request.",
		});

		expect(error.kind).toBe("cancelled");
		expect(error.title).toBe("Response stopped");
	});

	test("classifies an offline browser", () => {
		const error = buildChatMessageError(
			"bits",
			new TypeError("Failed to fetch"),
		);

		expect(error.kind).toBe("network");
		expect(error.title).toBe("Could not reach FlowPilot");
	});

	test("keeps external backend guidance and its verify command", () => {
		const error = buildChatMessageError(
			"claude-code",
			new Error("Claude Code CLI was not found while trying to spawn it"),
		);

		expect(error.kind).toBe("backend");
		expect(error.command).toBe("claude --version && claude doctor");
		expect(error.title).toContain("Claude Code");
	});

	test("redacts credentials out of the technical details", () => {
		const error = buildChatMessageError(
			"bits",
			new Error("upstream refused: Bearer abc.def.ghi"),
		);

		expect(error.detail ?? error.message).not.toContain("abc.def.ghi");
	});

	test("never leaves a failure without a usable card", () => {
		const error = buildChatMessageError("bits", undefined);

		expect(error.kind).toBe("generic");
		expect(error.title).toBe("FlowPilot could not finish this response");
		expect(error.message.length).toBeGreaterThan(0);
	});
});
