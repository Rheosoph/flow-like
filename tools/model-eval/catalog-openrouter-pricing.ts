type RecordValue = Record<string, unknown>;

export interface PricingBit {
	id: string;
	type: string;
	parameters?: RecordValue | null;
}

export type ReferenceDisplayPrice =
	| {
			kind: "tokens";
			input: number;
			output: number;
			request: number;
			fromBenchmark: false;
	  }
	| { kind: "image"; perImage: number }
	| { kind: "speech"; perMillion: number; unit: "characters" | "bytes" }
	| { kind: "transcription"; perMinute: number }
	| { kind: "embedding"; perMillion: number; unit: "tokens"; request: number };

export interface PricingReference {
	version: 1;
	source: "openrouter";
	status: "listed" | "unavailable";
	requested_model_id: string;
	resolved_model_id?: string;
	configured_model_ids?: string[];
	representative_model_id?: string;
	routing_verified: boolean;
	routing_checked_at?: string;
	checked_at: string;
	source_urls: string[];
	basis: string;
	caveats: string[];
	display?: ReferenceDisplayPrice;
	provider_options: {
		provider: string;
		model_id: string;
		display: ReferenceDisplayPrice;
		configuration?: RecordValue;
	}[];
	configuration?: RecordValue;
}

export interface PresetPriceTarget {
	preset: string;
	model_id: string;
	routing_verified: boolean;
	source_url: string;
	provider?: RecordValue;
	fallback_model_ids?: string[];
	checked_at?: string;
}

export interface OpenRouterPricingSnapshot {
	fetched_at: string;
	source_url: string;
	data: RecordValue[];
	sources: Record<string, { status?: number; data?: unknown; error?: string }>;
	audio_units?: Record<
		string,
		{ unit: string; source_url: string; price_text?: string }
	>;
}

export interface OpenRouterPricingPlan {
	bit_id: string;
	expected_model_id: string;
	pricing_reference: PricingReference;
	changed: boolean;
}

function record(value: unknown): RecordValue {
	return value && typeof value === "object" && !Array.isArray(value)
		? (value as RecordValue)
		: {};
}

function text(value: unknown): string | undefined {
	return typeof value === "string" && value.trim() ? value : undefined;
}

export function openRouterRate(value: unknown): number | null {
	if (
		typeof value !== "number" &&
		(typeof value !== "string" || !/^\d+(?:\.\d+)?(?:e[+-]?\d+)?$/i.test(value))
	)
		return null;
	const amount = Number(value);
	return Number.isFinite(amount) && amount >= 0 ? amount : null;
}

function list(value: unknown): RecordValue[] {
	return Array.isArray(value) ? value.map(record) : [];
}

function stable(value: unknown): string {
	if (Array.isArray(value)) return `[${value.map(stable).join(",")}]`;
	if (value && typeof value === "object")
		return `{${Object.entries(value)
			.filter(([, entry]) => entry !== undefined)
			.sort(([a], [b]) => a.localeCompare(b))
			.map(([key, entry]) => `${JSON.stringify(key)}:${stable(entry)}`)
			.join(",")}}`;
	return JSON.stringify(value) ?? "null";
}

function priceValue(price: ReferenceDisplayPrice): number {
	switch (price.kind) {
		case "tokens":
			return (
				(1_500 * price.input + 500 * price.output) / 1_000_000 + price.request
			);
		case "embedding":
			return (600 * price.perMillion) / 1_000_000 + price.request;
		case "image":
			return price.perImage;
		case "transcription":
			return price.perMinute;
		default:
			return price.perMillion;
	}
}

function withinPriceCaps(
	price: ReferenceDisplayPrice,
	policy: RecordValue,
): boolean {
	const caps = record(policy.max_price);
	for (const [key, value] of Object.entries(caps)) {
		if (!["prompt", "completion", "request"].includes(key)) continue;
		const cap = openRouterRate(value);
		const rate =
			price.kind === "tokens"
				? key === "prompt"
					? price.input
					: key === "completion"
						? price.output
						: price.request
				: price.kind === "embedding"
					? key === "prompt"
						? price.perMillion
						: key === "request"
							? price.request
							: null
					: null;
		if (cap === null || rate === null || rate > cap) return false;
	}
	return true;
}

function sourceData(
	snapshot: OpenRouterPricingSnapshot,
	url: string,
): RecordValue {
	const source = snapshot.sources[url];
	return source?.status === 200 ? record(source.data) : {};
}

function unitPrice(
	type: string,
	pricing: RecordValue,
	unit?: string,
): ReferenceDisplayPrice | null {
	const prompt = openRouterRate(pricing.prompt);
	const output = openRouterRate(pricing.completion);
	const request =
		pricing.request === undefined ? 0 : openRouterRate(pricing.request);
	if (prompt === null || request === null) return null;
	if (["Llm", "Vlm", "SystemOne"].includes(type))
		return output === null
			? null
			: {
					kind: "tokens",
					input: prompt * 1_000_000,
					output: output * 1_000_000,
					request,
					fromBenchmark: false,
				};
	if (type === "Embedding" || type === "ImageEmbedding")
		return {
			kind: "embedding",
			perMillion: prompt * 1_000_000,
			unit: "tokens",
			request,
		};
	if (type === "Tts" && (unit === "characters" || unit === "bytes"))
		return { kind: "speech", perMillion: prompt * 1_000_000, unit };
	if (type === "Stt") {
		const factor =
			unit === "seconds"
				? 60
				: unit === "minutes"
					? 1
					: unit === "hours"
						? 1 / 60
						: null;
		if (factor !== null)
			return { kind: "transcription", perMinute: prompt * factor };
	}
	return null;
}

function representativeImage(
	endpoint: RecordValue,
): { display: ReferenceDisplayPrice; configuration: RecordValue } | null {
	const rates = list(endpoint.pricing).filter(
		(rate) =>
			rate.billable === "output_image" &&
			openRouterRate(rate.cost_usd) !== null,
	);
	const candidates = rates
		.map((rate) => {
			const variant = text(rate.variant)?.toLowerCase();
			const priority =
				variant === "medium_1k" ? 0 : variant === "1k" ? 1 : !variant ? 2 : 99;
			return { rate, priority };
		})
		.filter((candidate) => candidate.priority < 99)
		.sort((a, b) => a.priority - b.priority);
	const rate = candidates[0]?.rate;
	if (!rate || !["image", "megapixel"].includes(String(rate.unit))) return null;
	const variant = text(rate.variant);
	const resolutions = record(
		record(endpoint.supported_parameters).resolution,
	).values;
	const configuration: RecordValue = { output_images: 1, input_references: 0 };
	if (rate.unit === "megapixel") configuration.output_megapixels = 1;
	else if (variant) {
		configuration.resolution = "1K";
		if (variant.toLowerCase().startsWith("medium_"))
			configuration.quality = "medium";
	} else if (Array.isArray(resolutions))
		configuration.resolution = resolutions.includes("1K")
			? "1K"
			: resolutions[0];
	configuration.label =
		rate.unit === "megapixel"
			? "1 output megapixel, no reference images"
			: `${configuration.quality ? "medium quality, " : ""}${configuration.resolution ? `${configuration.resolution} output, ` : "base output, "}no reference images`;
	return {
		display: { kind: "image", perImage: Number(rate.cost_usd) },
		configuration,
	};
}

/** Provider reference prices are separate from the tariffs used to reserve runtime usage. */
export function planOpenRouterPricing(
	bits: PricingBit[],
	snapshot: OpenRouterPricingSnapshot,
	presetTargets: PresetPriceTarget[] = [],
): OpenRouterPricingPlan[] {
	if (!Number.isFinite(Date.parse(snapshot.fetched_at)))
		throw new Error("Pricing snapshot requires a valid timestamp");
	const models = new Map(
		snapshot.data.map((model) => [String(model.id), model]),
	);
	if (models.size !== snapshot.data.length)
		throw new Error("Duplicate model identities in pricing snapshot");
	const presets = new Map(
		presetTargets.map((target) => [target.preset, target]),
	);
	if (presets.size !== presetTargets.length)
		throw new Error("Duplicate preset pricing targets");
	const ids = new Set<string>();
	return bits.flatMap((bit) => {
		if (ids.has(bit.id)) throw new Error(`Duplicate Bit ${bit.id}`);
		ids.add(bit.id);
		const parameters = record(bit.parameters);
		const provider = record(parameters.provider);
		if (provider.provider_name !== "hosted:openrouter") return [];
		const requested = text(provider.model_id);
		if (!requested) throw new Error(`Missing model identity for ${bit.id}`);
		const preset = requested.startsWith("@preset/")
			? presets.get(requested)
			: undefined;
		const resolved = requested.startsWith("@preset/")
			? preset?.model_id
			: requested;
		const reference: PricingReference = {
			version: 1,
			source: "openrouter",
			status: "unavailable",
			requested_model_id: requested,
			resolved_model_id: resolved,
			routing_verified: preset
				? preset.routing_verified
				: !requested.startsWith("@preset/"),
			checked_at: snapshot.fetched_at,
			source_urls: [snapshot.source_url],
			basis: "Current provider pricing is unavailable for this exact route.",
			caveats: [],
			provider_options: [],
		};
		const finish = () => [
			{
				bit_id: bit.id,
				expected_model_id: requested,
				pricing_reference: reference,
				changed: stable(parameters.pricing_reference) !== stable(reference),
			},
		];
		if (!resolved) {
			reference.basis =
				"This preset's current model and provider routing could not be verified.";
			return finish();
		}
		if (preset) {
			const presetSource = new URL(preset.source_url);
			presetSource.search = "";
			presetSource.hash = "";
			reference.source_urls.push(presetSource.href);
			if (preset.checked_at && Number.isFinite(Date.parse(preset.checked_at)))
				reference.routing_checked_at = preset.checked_at;
			if (!preset.routing_verified)
				reference.caveats.push(
					"Listed price for the intended model; this preset's current routing is unverified.",
				);
		}
		if (preset?.fallback_model_ids?.length) {
			if (!preset.routing_verified)
				throw new Error(
					`Fallback models require verified preset routing: ${requested}`,
				);
			const candidates = [resolved, ...preset.fallback_model_ids];
			if (
				new Set(candidates).size !== candidates.length ||
				candidates.some(
					(candidate) => !candidate || candidate.startsWith("@preset/"),
				)
			)
				throw new Error(`Invalid preset model candidates: ${requested}`);
			reference.configured_model_ids = candidates;
			reference.resolved_model_id = undefined;
			const candidateReferences = candidates.map(
				(candidate) =>
					planOpenRouterPricing([bit], snapshot, [
						{ ...preset, model_id: candidate, fallback_model_ids: undefined },
					])[0].pricing_reference,
			);
			const representative = candidateReferences.find(
				(candidate) => candidate.status === "listed",
			);
			reference.source_urls = [
				...new Set([
					...reference.source_urls,
					...candidateReferences.flatMap((candidate) => candidate.source_urls),
				]),
			];
			reference.provider_options = candidateReferences.flatMap(
				(candidate) => candidate.provider_options,
			);
			reference.caveats.push(
				`This preset tries ${candidates.join(", then ")} in order. The model that serves the request determines the charge; fallback prices can differ.`,
			);
			for (const candidate of candidateReferences) {
				if (candidate.status !== "listed")
					reference.caveats.push(
						`Pricing is unavailable for configured model ${candidate.resolved_model_id}: ${candidate.basis}`,
					);
			}
			if (representative) {
				reference.status = "listed";
				reference.display = representative.display;
				reference.configuration = representative.configuration;
				reference.representative_model_id = representative.resolved_model_id;
				reference.basis =
					representative.resolved_model_id === resolved
						? `Representative price for this preset's configured primary model, ${resolved}. ${representative.basis}`
						: `Illustrative price for configured fallback model ${representative.resolved_model_id}; the primary model ${resolved} could not be priced. ${representative.basis}`;
				reference.caveats.push(...representative.caveats);
			} else
				reference.basis =
					"No current price could be verified for this preset's configured models and provider restrictions.";
			return finish();
		}
		const model = models.get(resolved);
		if (!model) {
			reference.basis =
				"This exact model is absent from the current OpenRouter catalog.";
			return finish();
		}
		reference.source_urls.push(`https://openrouter.ai/${resolved}`);
		const modalities = record(model.architecture).output_modalities;
		const expected = {
			Tts: "speech",
			Stt: "transcription",
			Embedding: "embeddings",
			ImageEmbedding: "embeddings",
			ImageGeneration: "image",
			Llm: "text",
			Vlm: "text",
			SystemOne: "decisions",
		}[bit.type];
		if (
			!expected ||
			!Array.isArray(modalities) ||
			!modalities.includes(expected)
		) {
			reference.basis =
				"The model's current output modality does not match this catalog category.";
			return finish();
		}
		const endpointUrl =
			bit.type === "ImageGeneration"
				? `https://openrouter.ai/api/v1/images/models/${resolved}/endpoints`
				: `https://openrouter.ai${text(record(model.links).details) ?? `/api/v1/models/${resolved}/endpoints`}`;
		const envelope = sourceData(snapshot, endpointUrl);
		const endpointModel =
			bit.type === "ImageGeneration" ? envelope : record(envelope.data);
		reference.source_urls.push(endpointUrl);
		const audioUnit = snapshot.audio_units?.[resolved];
		if (["Tts", "Stt"].includes(bit.type)) {
			if (!audioUnit) {
				reference.basis =
					"The provider's audio billing unit has not been verified.";
				return finish();
			}
			reference.source_urls.push(audioUnit.source_url);
		}
		let endpoints = (
			endpointModel.id === resolved ? list(endpointModel.endpoints) : []
		).filter(
			(endpoint) => endpoint.status === undefined || endpoint.status === 0,
		);
		const activeEndpointCount = endpoints.length;
		const providerParams = record(provider.params);
		const policy = {
			...preset?.provider,
			...record(providerParams.provider),
			...record(record(providerParams.request_params).provider),
		};
		const allowed = Array.isArray(policy.only)
			? policy.only
			: policy.allow_fallbacks === false && Array.isArray(policy.order)
				? policy.order
				: undefined;
		const ignored = Array.isArray(policy.ignore) ? policy.ignore : [];
		if (
			Object.keys(policy).some(
				(key) =>
					!["only", "ignore", "order", "allow_fallbacks", "max_price"].includes(
						key,
					),
			) ||
			Object.keys(record(policy.max_price)).some(
				(key) => !["prompt", "completion", "request"].includes(key),
			)
		)
			reference.caveats.push(
				"This reference does not model every routing restriction or price cap; the runtime route can differ.",
			);
		if (allowed || ignored.length)
			endpoints = endpoints.filter((endpoint) => {
				const names = [
					endpoint.provider_name,
					endpoint.provider_slug,
					endpoint.provider_tag,
					endpoint.tag,
				]
					.filter((name): name is string => typeof name === "string")
					.map((name) => name.toLowerCase());
				return (
					(!allowed ||
						allowed.some(
							(name) =>
								typeof name === "string" && names.includes(name.toLowerCase()),
						)) &&
					!ignored.some(
						(name) =>
							typeof name === "string" && names.includes(name.toLowerCase()),
					)
				);
			});
		for (const endpoint of endpoints) {
			const image =
				bit.type === "ImageGeneration" ? representativeImage(endpoint) : null;
			const display =
				image?.display ??
				unitPrice(bit.type, record(endpoint.pricing), audioUnit?.unit);
			if (!display || !withinPriceCaps(display, policy)) continue;
			reference.provider_options.push({
				provider: String(
					endpoint.provider_name ?? endpoint.tag ?? "OpenRouter",
				),
				model_id: resolved,
				display,
				...(image ? { configuration: image.configuration } : {}),
			});
		}
		if (!reference.provider_options.length) {
			const unsupportedListingPolicy =
				Object.keys(policy).some(
					(key) =>
						![
							"only",
							"ignore",
							"order",
							"allow_fallbacks",
							"max_price",
							"sort",
							"require_parameters",
							"data_collection",
						].includes(key),
				) ||
				(policy.data_collection !== undefined &&
					policy.data_collection !== "allow") ||
				Object.keys(record(policy.max_price)).some(
					(key) => !["prompt", "completion", "request"].includes(key),
				);
			if (
				activeEndpointCount ||
				allowed ||
				ignored.length ||
				unsupportedListingPolicy
			) {
				reference.basis =
					"No valid published rate was available for the selected provider and billing unit.";
				return finish();
			}
			const listed = unitPrice(
				bit.type,
				record(model.pricing),
				audioUnit?.unit,
			);
			if (!listed || !withinPriceCaps(listed, policy)) {
				reference.basis =
					"No valid published rate was available for the selected provider and billing unit.";
				return finish();
			}
			reference.provider_options.push({
				provider: "OpenRouter model listing",
				model_id: resolved,
				display: listed,
			});
			reference.caveats.push(
				"No active provider endpoint was verified. This is a listed model price and does not establish current availability.",
			);
		}
		reference.provider_options = [
			...new Map(
				reference.provider_options.map((option) => [stable(option), option]),
			).values(),
		];
		reference.provider_options.sort(
			(a, b) =>
				priceValue(a.display) - priceValue(b.display) ||
				a.provider.localeCompare(b.provider),
		);
		reference.display = reference.provider_options[0].display;
		reference.configuration = reference.provider_options[0].configuration;
		reference.status = "listed";
		reference.basis = `OpenRouter listed price via ${reference.provider_options[0].provider}.`;
		if (reference.provider_options.length > 1)
			reference.caveats.push(
				"The example uses the lowest listed provider rate for this workload; routing and provider availability can change the charge.",
			);
		if (bit.type === "ImageGeneration")
			reference.caveats.push(
				"Image examples use the stated output configuration. Higher resolutions, editing inputs and optional features can cost more.",
			);
		else if (
			["Llm", "Vlm", "SystemOne", "Embedding", "ImageEmbedding"].includes(
				bit.type,
			)
		)
			reference.caveats.push(
				"Text-only, uncached input at the base context rate. Images, audio, caching, long contexts and tools can use different rates.",
			);
		else if (bit.type === "Stt")
			reference.caveats.push(
				"Plain transcription. Prompting, diarization or other provider options can use different rates.",
			);
		else if (audioUnit?.unit === "bytes")
			reference.caveats.push(
				"Input is billed by UTF-8 bytes. Characters can occupy more than one byte.",
			);
		reference.caveats.push(
			"Provider list prices are shown in USD and exclude Flow-Like platform fees. Your plan's usage charge and provider-reported settlement can differ.",
		);
		reference.source_urls = [...new Set(reference.source_urls)];
		return finish();
	});
}
