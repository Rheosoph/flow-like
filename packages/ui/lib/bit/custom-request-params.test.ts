import { describe, expect, test } from "bun:test";
import {
	parseCustomRequestParams,
	supportsCustomRequestParams,
	validateCustomRequestParams,
	withCustomRequestParams,
} from "./custom-request-params";
import {
	type IProviderDef,
	providerFieldValues,
} from "./external-model-providers";

describe("custom request parameters", () => {
	test("accepts nested provider JSON without coercing values", () => {
		const value = {
			max_tokens: 8192,
			temperature: 0.25,
			thinking: { type: "adaptive", display: null },
			feature: false,
			stops: ["END", 42, null, { stream: false }],
		};
		expect(parseCustomRequestParams(JSON.stringify(value))).toEqual({
			value,
			error: null,
		});
	});

	test("requires valid object JSON and reserves streaming mode", () => {
		expect(parseCustomRequestParams("{").error).toContain("valid JSON");
		for (const value of [null, true, 12, "text", []]) {
			expect(parseCustomRequestParams(JSON.stringify(value)).error).toContain(
				"JSON object",
			);
		}
		for (const stream of [false, true, null]) {
			expect(validateCustomRequestParams({ stream })).toContain(
				"app controls streaming",
			);
		}
	});

	test("editing and clearing overrides keeps other provider settings", () => {
		const provider: IProviderDef = {
			key: "anthropic",
			providerName: "custom:anthropic",
			label: "Anthropic",
			description: "",
			primary: true,
			fields: [
				{ key: "model_id", label: "Model ID" },
				{ key: "endpoint", label: "Endpoint" },
				{ key: "api_key", label: "API key", secret: true },
			],
		};
		const existing = {
			model_id: "old-model",
			endpoint: "https://old.example",
			api_key: "fixture-secret",
			headers: { "x-custom": "fixture-header" },
			request_params: { max_tokens: 8192, feature: false },
			future_connection_setting: { enabled: true },
		};
		const loaded = parseCustomRequestParams(
			JSON.stringify(existing.request_params, null, 2),
		);
		const connection = providerFieldValues(
			provider,
			{ model_id: "new-model", api_key: "replacement-secret", endpoint: "" },
			existing,
		);
		const saved = withCustomRequestParams(connection.params, loaded.value);
		expect(saved).toEqual({
			model_id: "new-model",
			request_params: existing.request_params,
			future_connection_setting: existing.future_connection_setting,
		});
		expect(connection.secrets).toEqual({
			api_key: "replacement-secret",
			headers: existing.headers,
		});
		const cleared = withCustomRequestParams(
			saved,
			parseCustomRequestParams(" \n ").value,
		);
		expect(cleared).not.toHaveProperty("request_params");
		expect(cleared.future_connection_setting).toEqual({ enabled: true });
		expect(existing.endpoint).toBe("https://old.example");
		expect(existing.request_params.max_tokens).toBe(8192);
	});

	test("only offers overrides for custom HTTP providers", () => {
		for (const provider of [
			"custom:anthropic",
			"custom:gemini",
			"custom:systemone",
			"custom:typesafe",
			"custom:lmstudio",
		])
			expect(supportsCustomRequestParams(provider)).toBe(true);
		for (const provider of [
			"Local",
			"MLX",
			"Hosted",
			"custom:claude-code",
			"custom:codex",
			"custom:github-copilot",
			"custom:microsoft-copilot",
		])
			expect(supportsCustomRequestParams(provider)).toBe(false);
	});
});
