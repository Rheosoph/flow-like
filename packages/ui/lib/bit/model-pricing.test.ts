import { describe, expect, test } from "bun:test";
import { type IBit, IBitTypes } from "../schema/bit/bit";
import {
	bitPrice,
	costExamples,
	formatUsd,
	priceTiers,
	priceValue,
} from "./model-pricing";

function bit(
	type: IBitTypes,
	parameters: Record<string, unknown>,
	extra: Partial<IBit> = {},
): IBit {
	return { id: "model", type, parameters, ...extra } as unknown as IBit;
}

describe("model pricing", () => {
	test("keeps model identities visible among a preset's provider options", () => {
		const display = {
			kind: "tokens",
			input: 1,
			output: 3,
			request: 0,
			fromBenchmark: false,
		};
		const price = bitPrice(
			bit(IBitTypes.Llm, {
				provider: {
					provider_name: "hosted:openrouter",
					model_id: "@preset/choice",
				},
				pricing_reference: {
					version: 1,
					source: "openrouter",
					status: "listed",
					requested_model_id: "@preset/choice",
					configured_model_ids: ["publisher/primary", "publisher/fallback"],
					routing_verified: true,
					checked_at: "2026-11-16T12:00:00Z",
					source_urls: ["https://openrouter.ai/api/v1/presets/choice"],
					basis: "Configured primary model reference.",
					caveats: [],
					display,
					provider_options: [
						{
							provider: "Shared provider",
							model_id: "publisher/primary",
							display,
						},
						{
							provider: "Shared provider",
							model_id: "publisher/fallback",
							display: { ...display, input: 0.5 },
						},
					],
				},
			}),
		);
		expect(price.reference?.providers.map((provider) => provider.name)).toEqual(
			[
				"publisher/primary via Shared provider",
				"publisher/fallback via Shared provider",
			],
		);
		expect(price).toMatchObject(display);
	});

	test("uses the verified representative image price without changing the runtime reservation", () => {
		const parameters = {
			provider: {
				provider_name: "hosted:openrouter",
				model_id: "publisher/image",
			},
			pricing: { request_micro_usd: 120000 },
			pricing_reference: {
				version: 1,
				source: "openrouter",
				status: "listed",
				requested_model_id: "publisher/image",
				routing_verified: true,
				checked_at: "2026-11-16T12:00:00Z",
				source_urls: ["https://openrouter.ai/publisher/image"],
				basis: "One output megapixel.",
				caveats: ["Provider rate; platform fees are separate."],
				display: { kind: "image", perImage: 0.03 },
				provider_options: [],
				configuration: { label: "1 output megapixel, no references" },
			},
		};
		const price = bitPrice(
			bit(IBitTypes.ImageGeneration, parameters),
			"hosted",
		);
		expect(price).toMatchObject({
			kind: "image",
			perImage: 0.03,
			reference: { configuration: "1 output megapixel, no references" },
		});
		expect(costExamples(price)[0].usd).toBeCloseTo(0.3);
		expect(parameters.pricing.request_micro_usd).toBe(120000);
		for (const invalid of [
			{ ...parameters.pricing_reference, status: "unavailable" },
			{
				...parameters.pricing_reference,
				requested_model_id: "publisher/other",
			},
			{
				...parameters.pricing_reference,
				display: { kind: "image", perImage: -1 },
			},
			{
				...parameters.pricing_reference,
				display: { kind: "tokens", input: 0, output: 0, request: 0 },
			},
		]) {
			expect(
				bitPrice(
					bit(IBitTypes.ImageGeneration, {
						...parameters,
						pricing_reference: invalid,
					}),
					"hosted",
				).kind,
			).toBe("unlisted");
		}
	});

	test("reads hosted token prices in micro-USD per million tokens", () => {
		const price = bitPrice(
			bit(IBitTypes.Llm, {
				provider: { provider_name: "hosted:openrouter" },
				pricing: {
					input_micro_usd_per_million_tokens: 1_250_000,
					output_micro_usd_per_million_tokens: 10_000_000,
				},
			}),
			"hosted",
		);
		expect(price).toEqual({
			kind: "tokens",
			input: 1.25,
			output: 10,
			request: 0,
			fromBenchmark: false,
		});
		expect(priceValue(price)).toBeCloseTo(3.4375);
	});

	test("falls back to the benchmark listing when the catalog sets no token price", () => {
		const price = bitPrice(
			bit(
				IBitTypes.Vlm,
				{ provider: { provider_name: "hosted:openrouter" } },
				{
					model_evaluation: {
						slug: "m",
						name: "M",
						creator_name: "L",
						creator_slug: "l",
						pricing: {
							price_1m_input_tokens: 0.2,
							price_1m_output_tokens: 0.8,
						},
					},
				},
			),
			"hosted",
		);
		expect(price).toMatchObject({
			kind: "tokens",
			input: 0.2,
			output: 0.8,
			fromBenchmark: true,
		});
	});

	test("prices each type in its own unit", () => {
		expect(
			bitPrice(
				bit(IBitTypes.ImageGeneration, {
					pricing: { request_micro_usd: 20_000 },
				}),
				"hosted",
			),
		).toEqual({ kind: "image", perImage: 0.02 });
		expect(
			bitPrice(
				bit(IBitTypes.Stt, {
					pricing: { input_micro_usd_per_million_seconds: 50_000_000 },
				}),
				"hosted",
			),
		).toEqual({ kind: "transcription", perMinute: 0.003 });
		expect(
			bitPrice(
				bit(IBitTypes.Tts, {
					pricing: { input_micro_usd_per_million_characters: 15_000_000 },
				}),
				"hosted",
			),
		).toEqual({ kind: "speech", perMillion: 15, unit: "characters" });
		expect(
			bitPrice(
				bit(IBitTypes.Embedding, {
					pricing: { input_micro_usd_per_million_bytes: 50_000 },
				}),
				"hosted",
			),
		).toEqual({
			kind: "embedding",
			perMillion: 0.05,
			unit: "bytes",
			request: 0,
		});
	});

	test("uses the API's default audio tariff for hosted audio without a price", () => {
		const hosted = { provider: { provider_name: "hosted:openai" } };
		expect(bitPrice(bit(IBitTypes.Stt, hosted), "hosted")).toEqual({
			kind: "transcription",
			perMinute: 0.006,
		});
		expect(bitPrice(bit(IBitTypes.Tts, hosted), "hosted")).toEqual({
			kind: "speech",
			perMillion: 30,
			unit: "characters",
		});
		expect(
			bitPrice(
				bit(IBitTypes.Stt, { provider: { provider_name: "openai" } }),
				"hosted",
			).kind,
		).toBe("unlisted");
	});

	test("local, flow-runtime and custom models carry no catalog price", () => {
		const paid = { pricing: { request_micro_usd: 20_000 } };
		expect(bitPrice(bit(IBitTypes.ImageGeneration, paid), "local")).toEqual({
			kind: "free",
			where: "device",
		});
		expect(bitPrice(bit(IBitTypes.Forecasting, {}), "native")).toEqual({
			kind: "free",
			where: "flows",
		});
		expect(
			bitPrice(
				bit(IBitTypes.Llm, { provider: { provider_name: "custom:anthropic" } }),
				"hosted",
			),
		).toEqual({ kind: "ownKey" });
		expect(
			bitPrice(
				bit(IBitTypes.ImageGeneration, { pricing: { request_micro_usd: -1 } }),
				"hosted",
			).kind,
		).toBe("unlisted");
	});

	test("ranks paid prices into quartiles within one type and unit", () => {
		const image = (id: string, perImage: number) => ({
			id,
			type: IBitTypes.ImageGeneration,
			price: { kind: "image" as const, perImage },
		});
		const tiers = priceTiers([
			image("a", 0.01),
			image("b", 0.02),
			image("c", 0.04),
			image("d", 0.08),
			{
				id: "free",
				type: IBitTypes.ImageGeneration,
				price: { kind: "free", where: "device" },
			},
			{
				id: "chat",
				type: IBitTypes.Llm,
				price: {
					kind: "tokens",
					input: 100,
					output: 100,
					request: 0,
					fromBenchmark: false,
				},
			},
		]);
		expect(["a", "b", "c", "d"].map((id) => tiers.get(id))).toEqual([
			1, 2, 3, 4,
		]);
		expect(tiers.has("free")).toBe(false);
		expect(tiers.get("chat")).toBe(1);
	});

	test("turns rates into everyday costs", () => {
		const reply = costExamples({
			kind: "tokens",
			input: 1.25,
			output: 10,
			request: 0,
			fromBenchmark: false,
		});
		expect(reply[0]).toEqual({ key: "chatReply", usd: 0.006875 });
		expect(reply[2].usd).toBeCloseTo(6.875);
		expect(
			costExamples({ kind: "transcription", perMinute: 0.006 })[0].usd,
		).toBeCloseTo(0.36);
		expect(costExamples({ kind: "free", where: "device" })).toEqual([]);
	});

	test("formats small prices without losing them", () => {
		expect(formatUsd(10)).toBe("$10.00");
		expect(formatUsd(0.02)).toBe("$0.02");
		expect(formatUsd(0.006875)).toBe("$0.0069");
		expect(formatUsd(1234)).toBe("$1,234");
		expect(formatUsd(0.00001)).toBe("< $0.0001");
		expect(formatUsd(0)).toBe("$0");
	});
});
