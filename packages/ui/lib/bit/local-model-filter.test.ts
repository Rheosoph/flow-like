import { describe, expect, test } from "bun:test";
import type { IBit } from "../schema";
import {
	filterHostableLlmModels,
	getLlmModelTier,
	isFreeLlmModel,
	isLocalLlmModel,
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
		for (const name of ["Local", "local", "Llama.cpp", "LLAMACPP", "Ollama"]) {
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

describe("filterHostableLlmModels", () => {
	const models = [bit("Local"), bit("Hosted"), bit("llamacpp"), bit("OpenAI")];

	test("returns the list unchanged when the host can run llama.cpp", () => {
		expect(filterHostableLlmModels(models, true)).toEqual(models);
	});

	test("drops local-only models when the host cannot run llama.cpp", () => {
		expect(filterHostableLlmModels(models, false)).toEqual([
			bit("Hosted"),
			bit("OpenAI"),
		]);
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
