import { describe, expect, test } from "bun:test";
import { getI18n } from "@flow-like/locales";
import { DeviceTunnelError } from "../../../../../lib/device-management/tunnel";
import { TUNNEL_HTTP_HEAD_STALLED } from "../../../../../lib/device-management/tunnel-http";
import type { DevicesT } from "../../primitives/area-context";
import { AnswerBrokeOffError, GatewayError } from "./gateway-client";
import {
	chatMeasureText,
	embedMeasureText,
	failureText,
	vectorPreview,
} from "./playground-copy";

const t = getI18n().getFixedT("en", "devices") as DevicesT;
const DEVICE = "edge-berlin-01";

describe("measurements", () => {
	test("a chat answer: first token, speed, the counts and the whole time", () => {
		expect(
			chatMeasureText(t, "en", {
				ttftMs: 190,
				tokensPerSecond: 86.43,
				promptTokens: 620,
				completionTokens: 390,
				totalMs: 4_800,
			}),
		).toBe(
			"First token 190 ms · 86.4 tok/s · 620 tokens in, 390 out · 4.8 s in all",
		);
		expect(chatMeasureText(t, "en", { completionTokens: 1, totalMs: 40 })).toBe(
			"1 token out · 40 ms in all",
		);
		expect(
			chatMeasureText(t, "en", {
				promptTokens: 1,
				completionTokens: 1_200,
				totalMs: 40,
			}),
		).toBe("1 token in, 1,200 out · 40 ms in all");
		expect(chatMeasureText(t, "en", { totalMs: 12 })).toBe("12 ms in all");
	});

	test("an embedding: size, tokens, time and input speed", () => {
		const measure = {
			dimensions: 768,
			preview: [0, 0.001, -0.25],
			promptTokens: 1,
			totalMs: 45,
			tokensPerSecond: 22.2,
		};
		expect(embedMeasureText(t, "en", measure)).toBe(
			"768 dimensions · 1 token · 45 ms · 22.2 tok/s",
		);
		expect(vectorPreview(t, measure)).toBe("[0.0000, 0.0010, -0.2500, …]");
	});
});

describe("failures", () => {
	test("the gateway's refusals by status, with the device's own sentence", () => {
		expect(failureText(t, new GatewayError(404, "No model x"), DEVICE)).toBe(
			"edge-berlin-01 doesn't host this model anymore. “No model x”",
		);
		expect(failureText(t, new GatewayError(403, undefined), DEVICE)).toBe(
			"Your access to edge-berlin-01 doesn't include using its models.",
		);
		expect(failureText(t, new GatewayError(429, undefined), DEVICE)).toContain(
			"Too many requests are waiting for edge-berlin-01.",
		);
		expect(failureText(t, new GatewayError(502, undefined), DEVICE)).toBe(
			"The model gateway on edge-berlin-01 answered with error 502.",
		);
	});

	test("a refused gateway stream says whether access or the model host is missing", () => {
		expect(
			failureText(t, new DeviceTunnelError("unauthorized", "no"), DEVICE),
		).toBe("Your access to edge-berlin-01 doesn't include using its models.");
		expect(
			failureText(t, new DeviceTunnelError("unsupported", "no"), DEVICE),
		).toBe("edge-berlin-01 isn't running its model host right now.");
	});

	test("an answer that broke off, or never started while the device loads the model, says so", () => {
		expect(failureText(t, new AnswerBrokeOffError(), DEVICE)).toBe(
			"The answer from edge-berlin-01 broke off before the model finished. Try again.",
		);
		expect(failureText(t, new Error(TUNNEL_HTTP_HEAD_STALLED), DEVICE)).toBe(
			"edge-berlin-01 didn't start answering. It may still be loading the model; try again in a few minutes.",
		);
	});

	test("anything else reads as the connection's failure, never as a raw message", () => {
		const text = failureText(
			t,
			new DeviceTunnelError("connection_closed", "socket gone"),
			DEVICE,
		);
		expect(text.length).toBeGreaterThan(10);
		expect(text).not.toContain("socket gone");
	});
});
