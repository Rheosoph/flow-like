import { describe, expect, test } from "bun:test";
import {
	type OpenRouterPricingSnapshot,
	type PricingBit,
	openRouterRate,
	planOpenRouterPricing,
} from "./catalog-openrouter-pricing";

function fixture(type = "Llm", unit?: string) {
	const model = "publisher/model";
	const modality = {
		Llm: "text",
		Tts: "speech",
		Stt: "transcription",
		ImageGeneration: "image",
		Embedding: "embeddings",
		SystemOne: "decisions",
	}[type];
	const endpointUrl = `https://openrouter.ai/api/v1/${type === "ImageGeneration" ? "images/" : ""}models/${model}/endpoints`;
	const bit: PricingBit = {
		id: "bit",
		type,
		parameters: {
			provider: { provider_name: "hosted:openrouter", model_id: model },
			pricing: { request_micro_usd: 500000 },
			context_length: 10000,
		},
	};
	const endpoints: Record<string, unknown>[] = [
		{
			provider_name: "Provider",
			tag: "provider",
			status: 0,
			pricing: { prompt: "0.000001", completion: "0.000003" },
		},
	];
	const endpointModel = { id: model, endpoints };
	const snapshot: OpenRouterPricingSnapshot = {
		fetched_at: "2026-11-16T12:00:00Z",
		source_url: "https://openrouter.ai/api/v1/models?output_modalities=all",
		data: [
			{
				id: model,
				architecture: { output_modalities: [modality] },
				links: { details: `/api/v1/models/${model}/endpoints` },
			},
		],
		sources: {
			[endpointUrl]: {
				status: 200,
				data:
					type === "ImageGeneration" ? endpointModel : { data: endpointModel },
			},
		},
		audio_units: unit
			? { [model]: { unit, source_url: `https://openrouter.ai/${model}` } }
			: {},
	};
	return { bit, snapshot, endpoints, endpointModel, endpointUrl };
}

describe("OpenRouter price references", () => {
	test("qualifies explicit model-list prices when no provider endpoint is active", () => {
		const { bit, snapshot, endpoints } = fixture();
		snapshot.data[0].pricing = { prompt: "0", completion: "0" };
		endpoints.splice(0);
		const reference = planOpenRouterPricing([bit], snapshot)[0]
			.pricing_reference;
		expect(reference.display).toMatchObject({ input: 0, output: 0 });
		expect(reference.caveats.join(" ")).toContain(
			"does not establish current availability",
		);
		snapshot.data[0].pricing = { prompt: "-1", completion: "-1" };
		expect(
			planOpenRouterPricing([bit], snapshot)[0].pricing_reference.status,
		).toBe("unavailable");
	});

	test("rejects absent, dynamic and invalid rates without turning them into free prices", () => {
		for (const value of [
			null,
			undefined,
			"",
			" ",
			"-1",
			-1,
			"NaN",
			Number.POSITIVE_INFINITY,
			{},
			"0x10",
		])
			expect(openRouterRate(value)).toBeNull();
		expect(openRouterRate("0")).toBe(0);
		expect(openRouterRate("1e-7")).toBe(1e-7);
		const { bit, snapshot, endpoints } = fixture();
		endpoints[0].pricing = { prompt: "-1", completion: "-1" };
		expect(
			planOpenRouterPricing([bit], snapshot)[0].pricing_reference.status,
		).toBe("unavailable");
	});

	test("does not replace an unavailable constrained provider with the unrestricted model-list price", () => {
		for (const policy of [
			{ only: ["missing"] },
			{ ignore: ["Provider"] },
			{ order: ["missing"], allow_fallbacks: false },
		]) {
			const { bit, snapshot, endpoints } = fixture();
			snapshot.data[0].pricing = { prompt: "0", completion: "0" };
			bit.parameters = {
				provider: {
					provider_name: "hosted:openrouter",
					model_id: "publisher/model",
					params: { request_params: { provider: policy } },
				},
			};
			expect(
				planOpenRouterPricing([bit], snapshot)[0].pricing_reference.status,
			).toBe("unavailable");
		}
	});

	test("keeps a verified free router list price while qualifying endpoint availability and enforcing caps", () => {
		const { bit, snapshot, endpoints } = fixture();
		endpoints.splice(0);
		snapshot.data[0].pricing = { prompt: "0", completion: "0" };
		const policy = {
			sort: "price",
			allow_fallbacks: true,
			require_parameters: true,
			data_collection: "allow",
			max_price: { completion: 12 },
		};
		bit.parameters = {
			provider: {
				provider_name: "hosted:openrouter",
				model_id: "publisher/model",
				params: { provider: policy },
			},
		};
		const reference = planOpenRouterPricing([bit], snapshot)[0]
			.pricing_reference;
		expect(reference.display).toMatchObject({ input: 0, output: 0 });
		expect(reference.caveats.join(" ")).toContain(
			"does not establish current availability",
		);
		snapshot.data[0].pricing = { prompt: "0", completion: "0.000013" };
		expect(
			planOpenRouterPricing([bit], snapshot)[0].pricing_reference.status,
		).toBe("unavailable");
	});

	test("includes request fees when selecting a provider for reply and page workloads", () => {
		for (const type of ["Llm", "Embedding", "SystemOne"]) {
			const { bit, snapshot, endpoints } = fixture(type);
			endpoints[0].provider_name = "Low token rate with fee";
			endpoints[0].pricing = { prompt: "0", completion: "0", request: "0.1" };
			endpoints.push({
				provider_name: "No request fee",
				status: 0,
				pricing: { prompt: "0.000001", completion: "0.000003", request: "0" },
			});
			expect(
				planOpenRouterPricing([bit], snapshot)[0].pricing_reference
					.provider_options[0].provider,
			).toBe("No request fee");
		}
	});

	test("selects a real provider price pair rather than combining independent minimum rates", () => {
		const { bit, snapshot, endpoints } = fixture();
		endpoints.splice(
			0,
			1,
			{
				provider_name: "Cheap input",
				status: 0,
				pricing: { prompt: "0.0000001", completion: "0.00001" },
			},
			{
				provider_name: "Balanced",
				status: 0,
				pricing: { prompt: "0.000001", completion: "0.000001" },
			},
		);
		const reference = planOpenRouterPricing([bit], snapshot)[0]
			.pricing_reference;
		expect(reference.display).toMatchObject({
			kind: "tokens",
			input: 1,
			output: 1,
		});
		expect(reference.provider_options[0].provider).toBe("Balanced");
		expect(reference.provider_options).toHaveLength(2);
		endpoints.push({ ...endpoints[1] });
		expect(
			planOpenRouterPricing([bit], snapshot)[0].pricing_reference
				.provider_options,
		).toHaveLength(2);
	});

	test("keeps seconds and hours distinct for transcription and bytes distinct from characters", () => {
		const hourly = fixture("Stt", "hours");
		hourly.endpoints[0].pricing = { prompt: "0.1", completion: "0" };
		expect(
			planOpenRouterPricing([hourly.bit], hourly.snapshot)[0].pricing_reference
				.display,
		).toEqual({ kind: "transcription", perMinute: 0.1 / 60 });
		const seconds = fixture("Stt", "seconds");
		seconds.endpoints[0].pricing = { prompt: "0.0001", completion: "0" };
		expect(
			planOpenRouterPricing([seconds.bit], seconds.snapshot)[0]
				.pricing_reference.display,
		).toEqual({ kind: "transcription", perMinute: 0.006 });
		const bytes = fixture("Tts", "bytes");
		bytes.endpoints[0].pricing = { prompt: "0.000015", completion: "0" };
		expect(
			planOpenRouterPricing([bytes.bit], bytes.snapshot)[0].pricing_reference
				.display,
		).toEqual({ kind: "speech", perMillion: 15, unit: "bytes" });
		bytes.snapshot.audio_units = {};
		expect(
			planOpenRouterPricing([bytes.bit], bytes.snapshot)[0].pricing_reference
				.status,
		).toBe("unavailable");
	});

	test("pairs the selected image price with that provider's output configuration", () => {
		const { bit, snapshot, endpoints } = fixture("ImageGeneration");
		endpoints.splice(
			0,
			1,
			{
				provider_name: "Large",
				pricing: [{ billable: "output_image", unit: "image", cost_usd: 0.2 }],
				supported_parameters: { resolution: { values: ["2K"] } },
			},
			{
				provider_name: "Megapixel",
				pricing: [
					{ billable: "output_image", unit: "megapixel", cost_usd: 0.03 },
					{ billable: "input_image", unit: "megapixel", cost_usd: 0.01 },
				],
			},
		);
		const reference = planOpenRouterPricing([bit], snapshot)[0]
			.pricing_reference;
		expect(reference.display).toEqual({ kind: "image", perImage: 0.03 });
		expect(reference.configuration).toMatchObject({
			output_megapixels: 1,
			input_references: 0,
		});
		expect(reference.configuration).not.toHaveProperty("resolution");
		expect(bit.parameters?.pricing).toEqual({ request_micro_usd: 500000 });
	});

	test("uses a named 1K medium image configuration and rejects unrecognized output units", () => {
		const { bit, snapshot, endpoints } = fixture("ImageGeneration");
		endpoints[0].pricing = [
			{
				billable: "output_image",
				unit: "image",
				cost_usd: 0.04,
				variant: "low_1k",
			},
			{
				billable: "output_image",
				unit: "image",
				cost_usd: 0.06,
				variant: "medium_1k",
			},
			{
				billable: "output_image",
				unit: "image",
				cost_usd: 0.08,
				variant: "medium_2k",
			},
		];
		const reference = planOpenRouterPricing([bit], snapshot)[0]
			.pricing_reference;
		expect(reference.display).toEqual({ kind: "image", perImage: 0.06 });
		expect(reference.configuration).toMatchObject({
			resolution: "1K",
			quality: "medium",
		});
		endpoints[0].pricing = [
			{ billable: "output_image", unit: "token", cost_usd: 0.001 },
		];
		expect(
			planOpenRouterPricing([bit], snapshot)[0].pricing_reference.status,
		).toBe("unavailable");
	});

	test("honors provider restrictions in runtime request parameters", () => {
		const { bit, snapshot, endpoints } = fixture();
		endpoints.push({
			provider_name: "Chosen",
			tag: "chosen",
			status: 0,
			pricing: { prompt: "0.000002", completion: "0.000004" },
		});
		bit.parameters = {
			...bit.parameters,
			provider: {
				provider_name: "hosted:openrouter",
				model_id: "publisher/model",
				params: { request_params: { provider: { only: ["chosen"] } } },
			},
		};
		const reference = planOpenRouterPricing([bit], snapshot)[0]
			.pricing_reference;
		expect(reference.provider_options).toHaveLength(1);
		expect(reference.provider_options[0].provider).toBe("Chosen");
	});

	test("requires a supplied preset identity and labels unverified linked-model references", () => {
		const { bit, snapshot } = fixture();
		bit.parameters = {
			...bit.parameters,
			provider: {
				provider_name: "hosted:openrouter",
				model_id: "@preset/choice",
			},
		};
		expect(
			planOpenRouterPricing([bit], snapshot)[0].pricing_reference.status,
		).toBe("unavailable");
		const reference = planOpenRouterPricing([bit], snapshot, [
			{
				preset: "@preset/choice",
				model_id: "publisher/model",
				routing_verified: false,
				source_url: "https://openrouter.ai/publisher/model",
			},
		])[0].pricing_reference;
		expect(reference.status).toBe("listed");
		expect(reference.routing_verified).toBe(false);
		expect(reference.caveats.join(" ")).toContain(
			"current routing is unverified",
		);
		expect(reference.requested_model_id).toBe("@preset/choice");
	});

	test("keeps verified primary and fallback prices distinct and removes private source queries", () => {
		const { bit, snapshot, endpoints } = fixture();
		bit.parameters = {
			provider: {
				provider_name: "hosted:openrouter",
				model_id: "@preset/choice",
			},
		};
		const fallback = "publisher/fallback";
		const fallbackUrl = `https://openrouter.ai/api/v1/models/${fallback}/endpoints`;
		snapshot.data.push({
			...snapshot.data[0],
			id: fallback,
			links: { details: `/api/v1/models/${fallback}/endpoints` },
		});
		snapshot.sources[fallbackUrl] = {
			status: 200,
			data: {
				data: {
					id: fallback,
					endpoints: [
						{
							provider_name: "Fallback provider",
							status: 0,
							pricing: { prompt: "0.0000001", completion: "0.0000001" },
						},
					],
				},
			},
		};
		const target = {
			preset: "@preset/choice",
			model_id: "publisher/model",
			fallback_model_ids: [fallback],
			routing_verified: true,
			source_url:
				"https://openrouter.ai/api/v1/presets/choice?workspace_id=private-id",
			checked_at: "2026-11-16T14:00:00Z",
			provider: { max_price: { completion: 12 } },
		};
		let reference = planOpenRouterPricing([bit], snapshot, [target])[0]
			.pricing_reference;
		expect(reference.configured_model_ids).toEqual([
			"publisher/model",
			fallback,
		]);
		expect(reference.resolved_model_id).toBeUndefined();
		expect(reference.representative_model_id).toBe("publisher/model");
		expect(reference.display).toMatchObject({ input: 1, output: 3 });
		expect(reference.provider_options).toHaveLength(2);
		expect(reference.basis).toContain("configured primary model");
		expect(reference.caveats.join(" ")).toContain("fallback prices can differ");
		expect(reference.source_urls.join(" ")).not.toContain("private-id");
		expect(reference.routing_checked_at).toBe(target.checked_at);
		endpoints[0].pricing = { prompt: "0.000001", completion: "0.000020" };
		reference = planOpenRouterPricing([bit], snapshot, [target])[0]
			.pricing_reference;
		expect(reference.representative_model_id).toBe(fallback);
		expect(reference.basis).toContain(
			"Illustrative price for configured fallback",
		);
		expect(reference.provider_options).toHaveLength(1);
		expect(() =>
			planOpenRouterPricing([bit], snapshot, [
				{ ...target, routing_verified: false },
			]),
		).toThrow("require verified");
	});

	test("honors per-million token and per-request caps, including runtime overrides", () => {
		for (const max_price of [
			{ prompt: "0.5" },
			{ completion: 2 },
			{ request: "0.005" },
			{ completion: -1 },
		]) {
			const { bit, snapshot, endpoints } = fixture();
			endpoints[0].pricing = {
				prompt: "0.000001",
				completion: "0.000003",
				request: "0.01",
			};
			bit.parameters = {
				provider: {
					provider_name: "hosted:openrouter",
					model_id: "@preset/choice",
					params: { request_params: { provider: { max_price } } },
				},
			};
			const reference = planOpenRouterPricing([bit], snapshot, [
				{
					preset: "@preset/choice",
					model_id: "publisher/model",
					routing_verified: true,
					source_url: "https://openrouter.ai/api/v1/presets/choice",
					provider: { max_price: { completion: 12 } },
				},
			])[0].pricing_reference;
			expect(reference.status).toBe("unavailable");
		}
	});

	test("does not borrow another model's endpoint record or cross model categories", () => {
		const { bit, snapshot, endpointModel } = fixture();
		endpointModel.id = "publisher/other";
		expect(
			planOpenRouterPricing([bit], snapshot)[0].pricing_reference.status,
		).toBe("unavailable");
		endpointModel.id = "publisher/model";
		bit.type = "Tts";
		expect(
			planOpenRouterPricing([bit], snapshot)[0].pricing_reference.status,
		).toBe("unavailable");
	});

	test("keeps runtime pricing untouched and remains idempotent after JSON serialization", () => {
		const { bit, snapshot } = fixture();
		const before = JSON.stringify(bit);
		const [plan] = planOpenRouterPricing([bit], snapshot);
		expect(JSON.stringify(bit)).toBe(before);
		const after = JSON.parse(
			JSON.stringify({
				...bit,
				parameters: {
					...bit.parameters,
					pricing_reference: plan.pricing_reference,
				},
			}),
		);
		expect(planOpenRouterPricing([after], snapshot)[0].changed).toBe(false);
		expect(
			planOpenRouterPricing(
				[{ ...bit, parameters: { provider: { provider_name: "Local" } } }],
				snapshot,
			),
		).toEqual([]);
	});
});
