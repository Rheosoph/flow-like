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
	test("routes a missing profile to the model settings", () => {
		const error = buildChatMessageError(
			"bits",
			apiFailure(400, {
				error: {
					code: "BAD_REQUEST",
					message:
						"No profile found for the owner. A synced profile with a language model is required",
				},
			}),
		);

		expect(error.kind).toBe("config");
		expect(error.title).toBe("No model configured");
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
					code: "PLAN_LIMIT_EXCEEDED",
					message:
						"Your Pro plan does not have enough hosted ai available for this action.",
				},
			}),
		);

		expect(error.kind).toBe("billing");
		expect(error.action?.kind).toBe("upgrade");
		expect(error.retryable).toBe(false);
	});

	test("an API from before the code gets the picker and loses its wrong sentence", () => {
		const blamedThePlan = buildChatMessageError(
			"bits",
			apiFailure(402, {
				error: {
					code: "PAYMENT_REQUIRED",
					message:
						"None of the models in this profile are included in your plan. Upgrade, or add a model your plan covers in Settings → Models.",
				},
			}),
		);
		const pointedAtSettings = buildChatMessageError(
			"bits",
			apiFailure(400, {
				error: {
					code: "BAD_REQUEST",
					message:
						"This profile has no language model. Add one in Settings → Models before using FlowPilot.",
				},
			}),
		);

		for (const error of [blamedThePlan, pointedAtSettings]) {
			expect(error.action).toEqual({
				kind: "add-model",
				label: "Choose a model",
				hostedOnly: true,
			});
			expect(error.message).toBe(
				"No language model in this profile can answer here.",
			);
		}
		expect(blamedThePlan.status).toBe(402);
	});

	test("a profile nothing in the cloud can run offers the model picker", () => {
		const error = buildChatMessageError(
			"bits",
			apiFailure(400, {
				error: {
					code: "PROFILE_MODEL_REQUIRED",
					message:
						"This profile has no language model that runs in the cloud. Its models run on your own devices. Add a hosted model to get an answer here.",
				},
			}),
		);

		expect(error.kind).toBe("config");
		expect(error.action).toEqual({
			kind: "add-model",
			label: "Choose a model",
			hostedOnly: true,
		});
		expect(error.message).toContain("own devices");
		expect(error.code).toBe("PROFILE_MODEL_REQUIRED");
		expect(error.retryable).toBe(false);
	});

	test("hosted models above the plan offer the picker too, not only the plans", () => {
		const error = buildChatMessageError(
			"bits",
			apiFailure(402, {
				error: {
					code: "PROFILE_MODEL_REQUIRED",
					message:
						"Your plan does not include the hosted language models in this profile. Add a model your plan covers, or upgrade.",
				},
			}),
		);

		expect(error.action?.kind).toBe("add-model");
		expect(error.status).toBe(402);
	});

	test("a desktop that can run none of the profile's models offers every model it could", () => {
		const error = buildChatMessageError(
			"bits",
			"Bot 'Scout' can't answer: no language model it can use runs on this desktop (No available model found in this profile). Add one in Settings → Models or pick one in the bot's settings.",
		);

		expect(error.action).toEqual({
			kind: "add-model",
			label: "Choose a model",
			hostedOnly: false,
		});
		expect(error.message).not.toContain("Settings");
		expect(error.detail).toContain("Scout");
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

	test("reads a native command rejection and names the chat that failed", () => {
		const error = buildChatMessageError(
			"bits",
			{ error: "Automation approval is required for this workflow revision." },
			"Support",
		);

		expect(error.kind).toBe("generic");
		expect(error.title).toBe("Support could not finish this response");
		expect(error.message).toBe(
			"Automation approval is required for this workflow revision.",
		);
	});

	test("never leaves a failure without a usable card", () => {
		const error = buildChatMessageError("bits", undefined);

		expect(error.kind).toBe("generic");
		expect(error.title).toBe("FlowPilot could not finish this response");
		expect(error.message.length).toBeGreaterThan(0);
	});
});
