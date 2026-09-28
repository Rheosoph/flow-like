import { describe, expect, test } from "bun:test";
import { type IBit, IBitTypes } from "../schema";
import {
	filterHostableLlmModels,
	getLlmModelTier,
	isFreeLlmModel,
	isHostableLlmModel,
	isHostedLlmModel,
	isHostedLlmProviderName,
	isLlamaCppLlmModel,
	isLocalLlmModel,
	isMlxLlmModel,
	selectProfileLlmModels,
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
		for (const name of ["Local", "local", "MLX"]) {
			expect(isLocalLlmModel(bit(name))).toBe(true);
		}
	});

	test("does not match hosted or remote providers", () => {
		for (const name of [
			"Hosted",
			"OpenAI",
			"Anthropic",
			"custom:ollama",
			"Llama.cpp",
			"LLAMACPP",
			"Ollama",
		]) {
			expect(isLocalLlmModel(bit(name))).toBe(false);
		}
	});

	test("does not match when provider metadata is absent", () => {
		expect(isLocalLlmModel(bit(undefined))).toBe(false);
	});
});

describe("local runtime providers", () => {
	test("native Claude Bits are only selectable on a host with agent CLI support", () => {
		const claude = bit("custom:claude-code");
		const desktop = {
			canHostLlamaCPP: true,
			canHostMLX: true,
			canUseNativeAgentProviders: true,
		};
		const browser = { canHostLlamaCPP: false, canHostMLX: false };
		expect(isHostableLlmModel(claude, desktop)).toBe(true);
		expect(isHostableLlmModel(claude, browser)).toBe(false);
		expect(
			filterHostableLlmModels(
				[
					claude,
					bit("custom:codex"),
					bit("custom:github-copilot"),
					bit("custom:microsoft-copilot"),
				],
				browser,
			).map((model) => model.id),
		).toEqual([
			"custom:codex",
			"custom:github-copilot",
			"custom:microsoft-copilot",
		]);
		expect(
			filterHostableLlmModels([claude], {
				canHostLlamaCPP: true,
				canHostMLX: true,
			}),
		).toEqual([]);
	});

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

	test("keeps embedded llama.cpp and endpoint-backed models on desktop", () => {
		expect(
			filterHostableLlmModels(models, {
				canHostLlamaCPP: true,
				canHostMLX: false,
			}),
		).toEqual([bit("Local"), bit("Hosted"), bit("llamacpp"), bit("OpenAI")]);
	});

	test("keeps MLX and endpoint-backed models on iOS", () => {
		expect(
			filterHostableLlmModels(models, {
				canHostLlamaCPP: false,
				canHostMLX: true,
			}),
		).toEqual([bit("Hosted"), bit("MLX"), bit("llamacpp"), bit("OpenAI")]);
	});

	test("drops embedded models on a remote-only host", () => {
		expect(
			filterHostableLlmModels(models, {
				canHostLlamaCPP: false,
				canHostMLX: false,
			}),
		).toEqual([bit("Hosted"), bit("llamacpp"), bit("OpenAI")]);
	});
});

describe("hosted provider aliases", () => {
	test("recognizes canonical and legacy hosted names", () => {
		for (const name of ["Hosted", "hosted:openai", "Premium", " internal "]) {
			expect(isHostedLlmProviderName(name)).toBe(true);
			expect(isHostedLlmModel(bit(name))).toBe(true);
		}
	});

	test("does not classify endpoint or embedded providers as hosted", () => {
		for (const name of ["Local", "MLX", "Ollama", "custom:openai"]) {
			expect(isHostedLlmProviderName(name)).toBe(false);
			expect(isHostedLlmModel(bit(name))).toBe(false);
		}
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

describe("selectProfileLlmModels", () => {
	const ALL_HOSTS = { canHostLlamaCPP: true, canHostMLX: true };
	const model = (id: string, hub: string, type = IBitTypes.Vlm): IBit =>
		({
			id,
			hub,
			type,
			parameters: { provider: { provider_name: "Hosted" } },
		}) as unknown as IBit;

	test("matches a profile reference whose hub differs from the bit's own hub", () => {
		// The Lean template references `api.flow-like.com:<id>` while the bit is
		// served carrying `api.alpha.flow-like.com` — it is still the same model.
		const free = model("ca6ziza1", "api.alpha.flow-like.com");
		const selected = selectProfileLlmModels(
			[free, model("other", "api.flow-like.com")],
			[],
			["api.flow-like.com:ca6ziza1"],
			ALL_HOSTS,
		);
		expect(selected.map((bit) => bit.id)).toEqual(["ca6ziza1"]);
	});

	test("matches bare references and ignores catalog models outside the profile", () => {
		const selected = selectProfileLlmModels(
			[model("a", "hub"), model("b", "hub")],
			[],
			["a"],
			ALL_HOSTS,
		);
		expect(selected.map((bit) => bit.id)).toEqual(["a"]);
	});

	test("adds custom models once and drops non-LLM bits", () => {
		const selected = selectProfileLlmModels(
			[model("shared", "hub"), model("embed", "hub", IBitTypes.Embedding)],
			[model("shared", "hub"), model("own", "hub", IBitTypes.Llm)],
			["hub:shared", "hub:embed"],
			ALL_HOSTS,
		);
		expect(selected.map((bit) => bit.id).sort()).toEqual(["own", "shared"]);
	});

	test("drops local models this host cannot run", () => {
		const local = {
			id: "local",
			hub: "hub",
			type: IBitTypes.Llm,
			parameters: { provider: { provider_name: "Local" } },
		} as unknown as IBit;
		const selected = selectProfileLlmModels([local], [], ["hub:local"], {
			canHostLlamaCPP: false,
			canHostMLX: false,
		});
		expect(selected).toEqual([]);
	});

	test("returns nothing until both catalog and profile have loaded", () => {
		expect(selectProfileLlmModels(undefined, [], ["a"], ALL_HOSTS)).toEqual([]);
		expect(
			selectProfileLlmModels([model("a", "hub")], [], undefined, ALL_HOSTS),
		).toEqual([]);
	});
});

describe("non-generation models on a host without local runtimes", () => {
	const browser = { canHostLlamaCPP: false, canHostMLX: false };

	function model(type: IBitTypes, remote?: boolean): IBit {
		return {
			id: `${type}-${remote ? "remote" : "local"}`,
			hub: "hub",
			type,
			parameters: {
				provider: { provider_name: "Local" },
				...(remote ? { remote: { implementation: "Internal" } } : {}),
			},
		} as unknown as IBit;
	}

	test("an embedding the hub can serve stays available", () => {
		expect(isHostableLlmModel(model(IBitTypes.Embedding, true), browser)).toBe(
			true,
		);
		expect(
			isHostableLlmModel(model(IBitTypes.ImageEmbedding, true), browser),
		).toBe(true);
	});

	test("an embedding with no remote implementation does not", () => {
		expect(isHostableLlmModel(model(IBitTypes.Embedding), browser)).toBe(false);
	});

	test("a local text model is still gated on its runtime", () => {
		expect(isHostableLlmModel(model(IBitTypes.Llm, true), browser)).toBe(false);
		expect(isHostableLlmModel(model(IBitTypes.Vlm, true), browser)).toBe(false);
	});
});
