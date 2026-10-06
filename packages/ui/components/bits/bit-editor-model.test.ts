import { describe, expect, test } from "bun:test";
import type { IBit } from "../../lib/schema/bit/bit";
import { IBitTypes } from "../../lib/schema/bit/bit";
import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";
import {
	clone,
	coreChanged,
	emptyMetadata,
	microUsdToUsd,
	saveAdminBit,
	splitBitSecrets,
	updateBitPricingField,
	usdToMicroUsd,
	validateBitDraft,
} from "./bit-editor-model";
function fixture(): IBit {
	return {
		id: "bit-one",
		authors: [],
		dependencies: [],
		dependency_tree_hash: "tree",
		hash: "hash",
		hub: "hub",
		type: IBitTypes.Llm,
		created: "now",
		updated: "now",
		parameters: {
			context_length: 2048,
			provider: {
				provider_name: "Hosted",
				api_surface: null,
				params: { tier: "team", custom: true },
			},
			unknown: { preserve: [1, 2] },
		},
		meta: {
			en: { ...emptyMetadata(), name: "One" },
			de: {
				...emptyMetadata(),
				name: "Eins",
				preview_media: ["https://example.invalid/old.png"],
			},
		},
	};
}
const profile = { id: "profile" } as IProfile;
function apiFixture(source: IBit) {
	const calls: string[] = [];
	const state = { failLanguage: "", noResult: false, newId: "" };
	const api = {
		stream: async (
			_profile: unknown,
			route: string,
			options: RequestInit,
			callback: (data: unknown) => void,
		) => {
			calls.push(route);
			callback({
				stage: "download",
				message: "Downloading artifact",
				percent: 50,
			});
			if (!state.noResult)
				callback({
					...JSON.parse(options.body as string),
					id: state.newId || source.id,
					meta: {},
				});
		},
		put: async (_profile: unknown, route: string) => {
			calls.push(route);
			if (route.endsWith(state.failLanguage) && state.failLanguage)
				throw new Error("Metadata failed");
		},
	} as IApiState;
	return { api, calls, state };
}
describe("bit editor persistence", () => {
	test("metadata-only changes save the changed locale without upserting the artifact", async () => {
		const original = fixture();
		const draft = clone(original);
		draft.meta.de.name = "Geändert";
		draft.meta.de.preview_media = [];
		const { api, calls } = apiFixture(original);
		const saved = await saveAdminBit(api, profile, original, draft);
		expect(calls).toEqual(["admin/bit/bit-one/de"]);
		expect(saved.meta.de.preview_media).toEqual([]);
		expect(saved.parameters).toEqual(original.parameters);
		expect(saved.meta.en).toEqual(original.meta.en);
	});
	test("retry skips a completed core save and successful locales", async () => {
		let persisted = fixture();
		const draft = clone(persisted);
		draft.version = "2";
		draft.meta.en.name = "New";
		draft.meta.de.name = "Neu";
		const { api, calls, state } = apiFixture(persisted);
		state.failLanguage = "/de";
		await expect(
			saveAdminBit(api, profile, persisted, draft, (bit) => {
				persisted = bit;
			}),
		).rejects.toThrow("Metadata failed");
		expect(persisted.version).toBe("2");
		expect(persisted.meta.en.name).toBe("New");
		state.failLanguage = "";
		await saveAdminBit(api, profile, persisted, draft);
		expect(calls).toEqual([
			"admin/bit/bit-one",
			"admin/bit/bit-one/en",
			"admin/bit/bit-one/de",
			"admin/bit/bit-one/de",
		]);
	});
	test("new artifact identities receive all locales and checkpoints keep pending locales on retry", async () => {
		let persisted = fixture();
		const draft = clone(persisted);
		draft.download_link = "https://example.invalid/new.gguf";
		const { api, state } = apiFixture(persisted);
		state.newId = "bit-two";
		state.failLanguage = "/de";
		await expect(
			saveAdminBit(api, profile, persisted, draft, (bit) => {
				persisted = bit;
			}),
		).rejects.toThrow();
		expect(persisted.id).toBe("bit-two");
		expect(persisted.meta.en.name).toBe("One");
		expect(persisted.meta.de).toBeUndefined();
	});
	test("incomplete streams never report saved or start metadata writes", async () => {
		const original = fixture();
		const draft = { ...original, version: "2" };
		const { api, calls, state } = apiFixture(original);
		state.noResult = true;
		await expect(saveAdminBit(api, profile, original, draft)).rejects.toThrow(
			"did not complete",
		);
		expect(calls).toHaveLength(1);
	});
	test("custom credentials are separated without modifying nested configuration or the source", () => {
		const original = fixture();
		original.parameters.provider.params.api_key = "fixture-key";
		original.parameters.provider.params.access_token = "fixture-access-token";
		original.parameters.provider.params.headers = {
			authorization: "fixture-value",
		};
		const { bit, secrets } = splitBitSecrets(original);
		expect(secrets).toEqual({
			api_key: "fixture-key",
			access_token: "fixture-access-token",
			headers: { authorization: "fixture-value" },
		});
		expect(bit.parameters.provider.params).toEqual({
			tier: "team",
			custom: true,
		});
		expect(bit.parameters.unknown).toEqual({ preserve: [1, 2] });
		expect(bit.parameters.provider.api_surface).toBeNull();
		expect(original.parameters.provider.params.api_key).toBe("fixture-key");
		expect(original.parameters.provider.params.access_token).toBe(
			"fixture-access-token",
		);
	});
	test("legacy metadata can be edited without normalizing untouched model parameters", () => {
		const original = fixture();
		const { context_length: _context, ...legacyParameters } =
			original.parameters;
		original.parameters = legacyParameters;
		const draft = clone(original);
		draft.meta.en.name = "Renamed";
		expect(validateBitDraft(draft, "admin", original)).toBeNull();
		expect(coreChanged(original, draft)).toBe(false);
		draft.parameters.context_length = -1;
		expect(validateBitDraft(draft, "admin", original)).toContain(
			"positive whole number",
		);
	});
	test("MLX dependencies differ for user manifests and registry bits", () => {
		const draft = fixture();
		draft.parameters.provider.provider_name = "MLX";
		expect(validateBitDraft(draft, "admin")).toContain("dependency");
		expect(validateBitDraft(draft, "custom")).toBeNull();
	});
	test("external model edits preserve fixed service IDs and text-only types", () => {
		const original = fixture();
		original.parameters.provider = {
			provider_name: "custom:microsoft-copilot",
			model_id: "microsoft-365-copilot",
			params: { model_id: "microsoft-365-copilot", timezone: "UTC" },
		};
		const draft = clone(original);
		expect(validateBitDraft(draft, "custom", original)).toBeNull();
		draft.parameters.provider.model_id = "arbitrary-model";
		expect(validateBitDraft(draft, "custom", original)).toContain(
			"fixed model ID",
		);
		draft.parameters.provider.model_id = "microsoft-365-copilot";
		draft.type = IBitTypes.Vlm;
		expect(validateBitDraft(draft, "custom", original)).toContain("text only");
		draft.parameters.provider = {
			provider_name: "custom:claude-code",
			model_id: "sonnet",
		};
		expect(validateBitDraft(draft, "custom", original)).toContain("text only");
		draft.type = IBitTypes.Llm;
		expect(validateBitDraft(draft, "custom", original)).toBeNull();
	});
	test("external model editing requires one consistent ID without requiring redacted credentials", () => {
		const original = fixture();
		const draft = clone(original);
		draft.parameters.provider = { provider_name: "custom:codex", params: {} };
		expect(validateBitDraft(draft, "custom", original)).toContain("model ID");
		draft.parameters.provider.model_id = "gpt-5.4";
		draft.parameters.provider.params.model_id = "gpt-5.3-codex";
		expect(validateBitDraft(draft, "custom", original)).toContain("must match");
		draft.parameters.provider.params.model_id = "gpt-5.4";
		expect(validateBitDraft(draft, "custom", original)).toBeNull();
	});
	test("hosted USD rates persist exactly and preserve unrelated parameters", async () => {
		const original = fixture();
		const draft = clone(original);
		draft.parameters.pricing = { future_field: "preserved" };
		draft.parameters = updateBitPricingField(
			draft.parameters,
			"input_micro_usd_per_million_tokens",
			"0.000001",
		);
		draft.parameters = updateBitPricingField(
			draft.parameters,
			"output_micro_usd_per_million_tokens",
			"9007199254.740991",
		);
		draft.parameters = updateBitPricingField(
			draft.parameters,
			"request_micro_usd",
			"0.000249",
		);
		expect(validateBitDraft(draft, "admin", original)).toBeNull();
		const { api, calls } = apiFixture(original);
		const saved = await saveAdminBit(api, profile, original, draft);
		expect(calls).toEqual(["admin/bit/bit-one"]);
		expect(saved.parameters.pricing).toEqual({
			input_micro_usd_per_million_tokens: 1,
			output_micro_usd_per_million_tokens: Number.MAX_SAFE_INTEGER,
			request_micro_usd: 249,
			future_field: "preserved",
		});
		expect(saved.parameters.provider).toEqual(original.parameters.provider);
		expect(saved.parameters.unknown).toEqual(original.parameters.unknown);
		expect(original.parameters.pricing).toBeUndefined();
	});
	test("absent or removed hosted pricing and an omitted request fee remain saveable", async () => {
		const original = fixture();
		expect(validateBitDraft(original, "admin")).toBeNull();
		original.parameters.pricing = null;
		expect(validateBitDraft(original, "admin")).toBeNull();
		original.parameters.pricing = {
			input_micro_usd_per_million_tokens: 0,
			output_micro_usd_per_million_tokens: 0,
			request_micro_usd: 1,
		};
		const draft = clone(original);
		draft.parameters = updateBitPricingField(
			draft.parameters,
			"request_micro_usd",
			"",
		);
		expect(draft.parameters.pricing.request_micro_usd).toBeUndefined();
		expect(validateBitDraft(draft, "admin")).toBeNull();
		const { pricing: _pricing, ...withoutPricing } = draft.parameters;
		draft.parameters = withoutPricing;
		expect(validateBitDraft(draft, "admin")).toBeNull();
		const { api } = apiFixture(original);
		const saved = await saveAdminBit(api, profile, original, draft);
		expect(saved.parameters.pricing).toBeUndefined();
		expect(saved.parameters.unknown).toEqual(original.parameters.unknown);
	});
	test("configured pricing rejects malformed or incomplete rates from fields and JSON", () => {
		const draft = fixture();
		for (const pricing of [
			{},
			[],
			"1",
			{ input_micro_usd_per_million_tokens: 0 },
			...[
				"1",
				-1,
				0.1,
				Number.NaN,
				Number.POSITIVE_INFINITY,
				Number.MAX_SAFE_INTEGER + 1,
			].map((rate) => ({
				input_micro_usd_per_million_tokens: rate,
				output_micro_usd_per_million_tokens: 0,
			})),
			{
				input_micro_usd_per_million_tokens: 0,
				output_micro_usd_per_million_tokens: 0,
				request_micro_usd: null,
			},
		]) {
			draft.parameters.pricing = pricing;
			expect(validateBitDraft(draft, "admin")).not.toBeNull();
		}
		draft.parameters.pricing = {
			input_micro_usd_per_million_tokens: 0,
			output_micro_usd_per_million_tokens: 0,
		};
		draft.parameters = updateBitPricingField(
			draft.parameters,
			"request_micro_usd",
			"0.0000001",
		);
		expect(validateBitDraft(draft, "admin")).toContain("USD per request");
	});
	test("USD conversion keeps micro-USD precision and rejects unsupported amounts", () => {
		for (const amount of [
			"0",
			"0.000001",
			"0.000249",
			"0.29",
			"3.141592",
			"9007199254.740991",
		]) {
			expect(microUsdToUsd(usdToMicroUsd(amount))).toBe(amount);
		}
		expect(usdToMicroUsd(".5")).toBe(500_000);
		expect(usdToMicroUsd("1.")).toBe(1_000_000);
		for (const invalid of [
			"",
			".",
			"-1",
			"NaN",
			"Infinity",
			"1e3",
			"0.0000001",
			"9007199254.740992",
		]) {
			expect(usdToMicroUsd(invalid)).toBeNull();
		}
	});
});

describe("hosted embedding validation", () => {
	function embedding(parameters: Record<string, unknown>) {
		return validateBitDraft(
			{
				...fixture(),
				type: IBitTypes.Embedding,
				parameters: { input_length: 2048, vector_length: 1024, ...parameters },
			},
			"admin",
		);
	}
	const remote = {
		implementation: "CloudflareWorkersAI",
		model_id: "@cf/qwen/qwen3-embedding-0.6b",
	};
	test("external providers require a model and input price while internal keeps its default pricing", () => {
		expect(embedding({ remote: {} })).toBeNull();
		expect(embedding({})).toBeNull();
		expect(embedding({ remote })).toContain("input price");
		expect(embedding({ remote: { implementation: "OpenAI" } })).toContain(
			"model or deployment ID",
		);
		expect(
			embedding({ remote, pricing: { input_micro_usd_per_million_tokens: 0 } }),
		).toBeNull();
		expect(
			embedding({
				remote,
				pricing: {
					input_micro_usd_per_million_tokens: 20,
					output_micro_usd_per_million_tokens: 0,
				},
			}),
		).toBeNull();
	});
	test("external model dimensions and input limits must match the hosted contract", () => {
		const pricing = { input_micro_usd_per_million_tokens: 0 };
		for (const input_length of [0, -1, 1.5, 4_294_967_296])
			expect(embedding({ remote, pricing, input_length })).toContain(
				"Input length",
			);
		const cohere = { implementation: "Cohere", model_id: "embed-v4.0" };
		expect(
			embedding({ remote: cohere, pricing, vector_length: 768 }),
		).toContain("256, 512, 1024, 1536");
		expect(
			embedding({
				remote: cohere,
				pricing,
				vector_length: 1024,
				input_length: 128000,
			}),
		).toBeNull();
		expect(
			embedding({
				remote: { implementation: "VoyageAI", model_id: "voyage-3.5" },
				pricing,
				vector_length: 1536,
			}),
		).toContain("vector dimensions");
		expect(
			embedding({
				remote: {
					implementation: "OpenAI",
					model_id: "text-embedding-3-small",
				},
				pricing,
				vector_length: 2048,
			}),
		).toContain("at most 1536");
	});
	test("byte pricing requires a batch limit and cannot coexist with token pricing", () => {
		const pricing = { input_micro_usd_per_million_bytes: 12 };
		expect(embedding({ remote, pricing })).toContain("batch byte limit");
		expect(
			embedding({ remote, pricing: { ...pricing, max_input_bytes: 4096 } }),
		).toBeNull();
		expect(
			embedding({
				remote,
				pricing: {
					...pricing,
					max_input_bytes: 4096,
					input_micro_usd_per_million_tokens: 12,
				},
			}),
		).toContain("exactly one");
	});
	test("Cloudflare BGE requires mean pooling to prevent mixing CLS and mean vectors", () => {
		const pricing = { input_micro_usd_per_million_tokens: 0 };
		const bge = { ...remote, model_id: "@cf/baai/bge-large-en-v1.5" };
		for (const pooling of ["CLS", "None", undefined]) {
			expect(embedding({ remote: bge, pricing, pooling })).toContain(
				"mean pooling",
			);
		}
		expect(embedding({ remote: bge, pricing, pooling: "Mean" })).toBeNull();
		expect(
			embedding({
				remote: { ...bge, implementation: "Internal" },
				pooling: "CLS",
			}),
		).toBeNull();
	});
	test("rejects invalid prices, ambiguous nulls, unsupported endpoints and output charges", () => {
		for (const pricing of [
			{ input_micro_usd_per_million_tokens: -1 },
			{ input_micro_usd_per_million_tokens: null },
			{ input_micro_usd_per_million_tokens: "0.0000011" },
			{ input_micro_usd_per_million_tokens: 1.5 },
			{ input_micro_usd_per_million_tokens: Number.MAX_SAFE_INTEGER + 1 },
			{ input_micro_usd_per_million_tokens: 0, request_micro_usd: null },
			{ input_micro_usd_per_million_tokens: 0, max_input_bytes: 0 },
			{
				input_micro_usd_per_million_tokens: 0,
				output_micro_usd_per_million_tokens: 10,
			},
		])
			expect(embedding({ remote, pricing })).not.toBeNull();
		expect(
			embedding({
				remote: { ...remote, endpoint_secret_name: "ENDPOINT" },
				pricing: { input_micro_usd_per_million_tokens: 0 },
			}),
		).toContain("fixed endpoint");
	});
});

test("SystemOne custom editing validates the native provider and separates credentials", () => {
	const draft = fixture();
	draft.type = IBitTypes.SystemOne;
	draft.parameters = {
		context_length: 512,
		provider: {
			provider_name: "custom:systemone",
			model_id: "decision",
			params: { endpoint: "https://models.example.test", api_key: "secret" },
		},
	};
	expect(validateBitDraft(draft, "custom")).toBeNull();
	const split = splitBitSecrets(draft);
	expect(JSON.stringify(split.bit)).not.toContain("secret");
	expect(JSON.stringify(split.secrets)).toContain("secret");
	draft.parameters.provider.provider_name = "custom:openai";
	expect(validateBitDraft(draft, "custom")).toContain("Chat providers");
});
