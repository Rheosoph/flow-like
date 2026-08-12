import { describe, expect, test } from "bun:test";
import type { IBit } from "../schema";
import {
	filterHostableLlmModels,
	getLlmModelTier,
	isFreeLlmModel,
	isHostableLlmModel,
	isLlamaCppLlmModel,
	isLocalLlmModel,
	isMlxLlmModel,
} from "./local-model-filter";
function bit(providerName?: string): IBit {
	return {
		id: providerName ?? "no-provider",
		hub: "hub",
		parameters:
			providerName === undefined
				? {}
				: { provider: { provider_name: providerName } },
	} as unknown as IBit;
}

describe("isLocalLlmModel", () => {
	test("matches local provider names case-insensitively", () => {
		for (const name of [
			"Local",
			"local",
			"Llama.cpp",
			"LLAMACPP",
			"Ollama",
			"MLX",
		]) {
			expect(isLocalLlmModel(bit(name))).toBe(true);
		}
	});

	test("does not match hosted or remote providers", () => {
		for (const name of ["Hosted", "OpenAI", "Anthropic", "custom:ollama"]) {
			expect(isLocalLlmModel(bit(name))).toBe(false);
		}
	});

	test("does not match when provider metadata is absent", () => {
		expect(isLocalLlmModel(bit(undefined))).toBe(false);
	});
});

describe("local runtime providers", () => {
	test("distinguishes llama.cpp-compatible and MLX models", () => {
		expect(isLlamaCppLlmModel(bit("Local"))).toBe(true);
		expect(isLlamaCppLlmModel(bit("MLX"))).toBe(false);
		expect(isMlxLlmModel(bit("mlx"))).toBe(true);
		expect(isMlxLlmModel(bit("llama.cpp"))).toBe(false);
	});

	test("checks the capability for the model's runtime", () => {
		const llamaOnly = { canHostLlamaCPP: true, canHostMLX: false };
		const mlxOnly = { canHostLlamaCPP: false, canHostMLX: true };

		expect(isHostableLlmModel(bit("Local"), llamaOnly)).toBe(true);
		expect(isHostableLlmModel(bit("MLX"), llamaOnly)).toBe(false);
		expect(isHostableLlmModel(bit("Local"), mlxOnly)).toBe(false);
		expect(isHostableLlmModel(bit("MLX"), mlxOnly)).toBe(true);
		expect(isHostableLlmModel(bit("OpenAI"), mlxOnly)).toBe(true);
	});
});

describe("filterHostableLlmModels", () => {
	const models = [
		bit("Local"),
		bit("Hosted"),
		bit("MLX"),
		bit("llamacpp"),
		bit("OpenAI"),
	];

	test("returns the list unchanged when the host can run both runtimes", () => {
		expect(
			filterHostableLlmModels(models, {
				canHostLlamaCPP: true,
				canHostMLX: true,
			}),
		).toEqual(models);
	});

	test("keeps only llama.cpp models on a non-Apple desktop", () => {
		expect(
			filterHostableLlmModels(models, {
				canHostLlamaCPP: true,
				canHostMLX: false,
			}),
		).toEqual([bit("Local"), bit("Hosted"), bit("llamacpp"), bit("OpenAI")]);
	});

	test("keeps only MLX models on iOS", () => {
		expect(
			filterHostableLlmModels(models, {
				canHostLlamaCPP: false,
				canHostMLX: true,
			}),
		).toEqual([bit("Hosted"), bit("MLX"), bit("OpenAI")]);
	});

	test("drops all local models on a remote-only host", () => {
		expect(
			filterHostableLlmModels(models, {
				canHostLlamaCPP: false,
				canHostMLX: false,
			}),
		).toEqual([bit("Hosted"), bit("OpenAI")]);
	});
});

describe("hosted model tiers", () => {
	test("normalizes and identifies the free tier", () => {
		const freeModel = bit("Hosted");
		freeModel.parameters.provider.params = { tier: " free " };

		expect(getLlmModelTier(freeModel)).toBe("FREE");
		expect(isFreeLlmModel(freeModel)).toBe(true);
	});

	test("does not treat an unspecified or paid tier as free", () => {
		const unspecified = bit("Hosted");
		const paid = bit("Hosted");
		paid.parameters.provider.params = { tier: "PRO" };

		expect(getLlmModelTier(unspecified)).toBeUndefined();
		expect(isFreeLlmModel(unspecified)).toBe(false);
		expect(isFreeLlmModel(paid)).toBe(false);
	});
});
