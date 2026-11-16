import { type IBit, IBitTypes } from "../schema/bit/bit";
import { isHostedAudio } from "./hosted-audio";

/** Where a model runs decides whether it costs anything per call. */
export type PriceDeployment = "hosted" | "local" | "native";

/** List prices in USD. Token, character and byte rates are per million units. */
type ModelPriceValue =
	| { kind: "free"; where: "device" | "flows" }
	| { kind: "ownKey" }
	| {
			kind: "tokens";
			input: number;
			output: number;
			request: number;
			/** Taken from the benchmark listing because the catalog sets no price. */
			fromBenchmark: boolean;
	  }
	| { kind: "image"; perImage: number }
	| { kind: "video"; perClip: number }
	| { kind: "speech"; perMillion: number; unit: "characters" | "bytes" }
	| { kind: "transcription"; perMinute: number }
	| {
			kind: "embedding";
			perMillion: number;
			unit: "tokens" | "bytes";
			request: number;
	  }
	| { kind: "unlisted" };

export interface ModelPriceReference {
	basis: string;
	caveats: string[];
	sources: string[];
	checkedAt: string;
	routingVerified: boolean;
	configuration?: string;
	providers: { name: string; price: ModelPriceValue }[];
}

export type ModelPrice = ModelPriceValue & { reference?: ModelPriceReference };

/** What the API meters hosted audio at when its Bit sets no price (`routes/audio/metering.rs`). */
const DEFAULT_TRANSCRIPTION_USD_PER_MINUTE = 0.006;
const DEFAULT_SPEECH_USD_PER_MILLION_CHARACTERS = 30;

const TOKEN_TYPES = new Set<IBitTypes>([
	IBitTypes.Llm,
	IBitTypes.Vlm,
	IBitTypes.SystemOne,
]);

function record(value: unknown): Record<string, unknown> {
	return value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: {};
}

/** Micro-USD integer to USD; anything else is not a price. */
function usd(value: unknown): number | null {
	return typeof value === "number" && Number.isFinite(value) && value >= 0
		? value / 1_000_000
		: null;
}

function providerName(bit: IBit): string {
	const name = record(record(bit.parameters).provider).provider_name;
	return typeof name === "string" ? name.trim().toLowerCase() : "";
}

function referenceDisplay(
	value: unknown,
	type: IBitTypes,
): ModelPriceValue | null {
	const price = record(value);
	const valid = (...values: unknown[]) =>
		values.every(
			(amount) =>
				typeof amount === "number" && Number.isFinite(amount) && amount >= 0,
		);
	if (
		TOKEN_TYPES.has(type) &&
		price.kind === "tokens" &&
		valid(price.input, price.output, price.request)
	)
		return {
			kind: "tokens",
			input: price.input as number,
			output: price.output as number,
			request: price.request as number,
			fromBenchmark: false,
		};
	if (
		type === IBitTypes.ImageGeneration &&
		price.kind === "image" &&
		valid(price.perImage)
	)
		return { kind: "image", perImage: price.perImage as number };
	if (
		type === IBitTypes.Tts &&
		price.kind === "speech" &&
		valid(price.perMillion) &&
		["characters", "bytes"].includes(String(price.unit))
	)
		return {
			kind: "speech",
			perMillion: price.perMillion as number,
			unit: price.unit as "characters" | "bytes",
		};
	if (
		type === IBitTypes.Stt &&
		price.kind === "transcription" &&
		valid(price.perMinute)
	)
		return { kind: "transcription", perMinute: price.perMinute as number };
	if (
		[IBitTypes.Embedding, IBitTypes.ImageEmbedding].includes(type) &&
		price.kind === "embedding" &&
		valid(price.perMillion, price.request) &&
		["tokens", "bytes"].includes(String(price.unit))
	)
		return {
			kind: "embedding",
			perMillion: price.perMillion as number,
			request: price.request as number,
			unit: price.unit as "tokens" | "bytes",
		};
	return null;
}

function referencePrice(bit: IBit, value: unknown): ModelPrice {
	const reference = record(value);
	const provider = record(record(bit.parameters).provider);
	if (
		reference.version !== 1 ||
		reference.source !== "openrouter" ||
		reference.requested_model_id !== provider.model_id ||
		typeof reference.basis !== "string" ||
		typeof reference.checked_at !== "string" ||
		!Number.isFinite(Date.parse(reference.checked_at)) ||
		!Array.isArray(reference.source_urls)
	)
		return { kind: "unlisted" };
	const sources = reference.source_urls.filter((source): source is string => {
		if (typeof source !== "string") return false;
		try {
			const url = new URL(source);
			return url.protocol === "https:" && !url.username && !url.password;
		} catch {
			return false;
		}
	});
	if (!sources.length) return { kind: "unlisted" };
	const details: ModelPriceReference = {
		basis: reference.basis,
		caveats: Array.isArray(reference.caveats)
			? reference.caveats.filter(
					(value): value is string => typeof value === "string",
				)
			: [],
		sources,
		checkedAt: reference.checked_at,
		routingVerified: reference.routing_verified === true,
		configuration:
			typeof record(reference.configuration).label === "string"
				? (record(reference.configuration).label as string)
				: undefined,
		providers: Array.isArray(reference.provider_options)
			? reference.provider_options.flatMap((option) => {
					const row = record(option);
					const price = referenceDisplay(row.display, bit.type);
					return price && typeof row.provider === "string"
						? [
								{
									name:
										Array.isArray(reference.configured_model_ids) &&
										reference.configured_model_ids.length > 1 &&
										typeof row.model_id === "string"
											? `${row.model_id} via ${row.provider}`
											: row.provider,
									price,
								},
							]
						: [];
				})
			: [],
	};
	const display =
		reference.status === "listed"
			? referenceDisplay(reference.display, bit.type)
			: null;
	return {
		...(display ?? { kind: "unlisted" }),
		reference: details,
	} as ModelPrice;
}

function tokenPrice(bit: IBit, pricing: Record<string, unknown>): ModelPrice {
	const input = usd(pricing.input_micro_usd_per_million_tokens);
	const output = usd(pricing.output_micro_usd_per_million_tokens);
	if (input !== null && output !== null) {
		return {
			kind: "tokens",
			input,
			output,
			request: usd(pricing.request_micro_usd) ?? 0,
			fromBenchmark: false,
		};
	}
	const listed = record(bit.model_evaluation?.pricing);
	const listedInput = listed.price_1m_input_tokens;
	const listedOutput = listed.price_1m_output_tokens;
	if (
		typeof listedInput === "number" &&
		typeof listedOutput === "number" &&
		listedInput >= 0 &&
		listedOutput >= 0
	) {
		return {
			kind: "tokens",
			input: listedInput,
			output: listedOutput,
			request: 0,
			fromBenchmark: true,
		};
	}
	return { kind: "unlisted" };
}

export function bitPrice(bit: IBit, deployment: PriceDeployment): ModelPrice {
	if (providerName(bit).startsWith("custom:")) return { kind: "ownKey" };
	if (deployment === "native") return { kind: "free", where: "flows" };
	if (deployment === "local") return { kind: "free", where: "device" };

	const parameters = record(bit.parameters);
	if (Object.hasOwn(parameters, "pricing_reference"))
		return referencePrice(bit, parameters.pricing_reference);
	const pricing = record(parameters.pricing);
	if (TOKEN_TYPES.has(bit.type)) return tokenPrice(bit, pricing);

	switch (bit.type) {
		case IBitTypes.ImageGeneration: {
			const perImage = usd(pricing.request_micro_usd);
			return perImage === null
				? { kind: "unlisted" }
				: { kind: "image", perImage };
		}
		case IBitTypes.VideoGeneration: {
			const perClip = usd(pricing.request_micro_usd);
			return perClip === null
				? { kind: "unlisted" }
				: { kind: "video", perClip };
		}
		case IBitTypes.Embedding:
		case IBitTypes.ImageEmbedding: {
			const request = usd(pricing.request_micro_usd) ?? 0;
			const tokens = usd(pricing.input_micro_usd_per_million_tokens);
			if (tokens !== null)
				return {
					kind: "embedding",
					perMillion: tokens,
					unit: "tokens",
					request,
				};
			const bytes = usd(pricing.input_micro_usd_per_million_bytes);
			if (bytes !== null)
				return { kind: "embedding", perMillion: bytes, unit: "bytes", request };
			return { kind: "unlisted" };
		}
		case IBitTypes.Tts: {
			const characters = usd(pricing.input_micro_usd_per_million_characters);
			if (characters !== null)
				return { kind: "speech", perMillion: characters, unit: "characters" };
			const bytes = usd(pricing.input_micro_usd_per_million_bytes);
			if (bytes !== null)
				return { kind: "speech", perMillion: bytes, unit: "bytes" };
			return isHostedAudio(bit)
				? {
						kind: "speech",
						perMillion: DEFAULT_SPEECH_USD_PER_MILLION_CHARACTERS,
						unit: "characters",
					}
				: { kind: "unlisted" };
		}
		case IBitTypes.Stt: {
			const perMillionSeconds = usd(
				pricing.input_micro_usd_per_million_seconds,
			);
			if (perMillionSeconds !== null)
				return {
					kind: "transcription",
					perMinute: (perMillionSeconds * 60) / 1_000_000,
				};
			return isHostedAudio(bit)
				? {
						kind: "transcription",
						perMinute: DEFAULT_TRANSCRIPTION_USD_PER_MINUTE,
					}
				: { kind: "unlisted" };
		}
		default:
			return { kind: "unlisted" };
	}
}

/** One number per price, comparable only between models of the same type. */
export function priceValue(price: ModelPrice): number | null {
	switch (price.kind) {
		case "free":
			return 0;
		case "tokens":
			return (3 * price.input + price.output) / 4;
		case "image":
			return price.perImage;
		case "video":
			return price.perClip;
		case "speech":
		case "embedding":
			return price.perMillion;
		case "transcription":
			return price.perMinute;
		default:
			return null;
	}
}

export type PriceTier = 1 | 2 | 3 | 4;

/**
 * Quartile of each paid model's price among paid models of the same type and unit,
 * so a tier never compares an image price with a token price.
 */
export function priceTiers(
	entries: ReadonlyArray<{ id: string; type: IBitTypes; price: ModelPrice }>,
): Map<string, PriceTier> {
	const groups = new Map<string, { id: string; value: number }[]>();
	for (const { id, type, price } of entries) {
		const value = priceValue(price);
		if (value === null || value <= 0) continue;
		const unit = "unit" in price ? price.unit : "";
		const key = `${type}:${price.kind}:${unit}`;
		const group = groups.get(key) ?? [];
		group.push({ id, value });
		groups.set(key, group);
	}
	const tiers = new Map<string, PriceTier>();
	for (const group of groups.values()) {
		const values = group.map((entry) => entry.value).sort((a, b) => a - b);
		for (const { id, value } of group) {
			const rank = values.indexOf(value);
			tiers.set(
				id,
				Math.min(4, 1 + Math.floor((4 * rank) / values.length)) as PriceTier,
			);
		}
	}
	return tiers;
}

export type CostExampleKey =
	| "chatReply"
	| "longDocument"
	| "thousandReplies"
	| "tenImages"
	| "hundredImages"
	| "thousandImages"
	| "shortClip"
	| "longClip"
	| "hundredClips"
	| "speechMinute"
	| "speechHour"
	| "speechHundredHours"
	| "audioHour"
	| "audioTenHours"
	| "audioHundredHours"
	| "thousandPages"
	| "hundredThousandPages";

export interface CostExample {
	key: CostExampleKey;
	usd: number;
}

/** Assumptions behind the examples, shown next to them. */
export const COST_ASSUMPTIONS = {
	replyInputTokens: 1_500,
	replyOutputTokens: 500,
	documentInputTokens: 35_000,
	documentOutputTokens: 1_500,
	speechCharactersPerMinute: 900,
	speechAsciiBytesPerMinute: 900,
	tokensPerPage: 600,
	bytesPerPage: 3_000,
} as const;

/** What everyday jobs cost at this price, so a rate per million reads as money. */
export function costExamples(price: ModelPrice): CostExample[] {
	const a = COST_ASSUMPTIONS;
	switch (price.kind) {
		case "tokens": {
			const reply =
				(a.replyInputTokens * price.input +
					a.replyOutputTokens * price.output) /
					1_000_000 +
				price.request;
			const document =
				(a.documentInputTokens * price.input +
					a.documentOutputTokens * price.output) /
					1_000_000 +
				price.request;
			return [
				{ key: "chatReply", usd: reply },
				{ key: "longDocument", usd: document },
				{ key: "thousandReplies", usd: reply * 1_000 },
			];
		}
		case "image":
			return [
				{ key: "tenImages", usd: price.perImage * 10 },
				{ key: "hundredImages", usd: price.perImage * 100 },
				{ key: "thousandImages", usd: price.perImage * 1_000 },
			];
		case "video":
			return [
				{ key: "shortClip", usd: price.perClip },
				{ key: "longClip", usd: price.perClip * 10 },
				{ key: "hundredClips", usd: price.perClip * 100 },
			];
		case "speech": {
			const minute =
				((price.unit === "bytes"
					? a.speechAsciiBytesPerMinute
					: a.speechCharactersPerMinute) *
					price.perMillion) /
				1_000_000;
			return [
				{ key: "speechMinute", usd: minute },
				{ key: "speechHour", usd: minute * 60 },
				{ key: "speechHundredHours", usd: minute * 6_000 },
			];
		}
		case "transcription":
			return [
				{ key: "audioHour", usd: price.perMinute * 60 },
				{ key: "audioTenHours", usd: price.perMinute * 600 },
				{ key: "audioHundredHours", usd: price.perMinute * 6_000 },
			];
		case "embedding": {
			const perPage =
				((price.unit === "tokens" ? a.tokensPerPage : a.bytesPerPage) *
					price.perMillion) /
					1_000_000 +
				price.request;
			return [
				{ key: "thousandPages", usd: perPage * 1_000 },
				{ key: "hundredThousandPages", usd: perPage * 100_000 },
			];
		}
		default:
			return [];
	}
}

/** Enough digits that small prices stay distinguishable, never trailing noise. */
export function formatUsd(value: number, locale = "en-US"): string {
	if (value > 0 && value < 0.0001) return `< ${formatUsd(0.0001, locale)}`;
	const digits =
		value === 0 || value >= 100 ? 0 : value >= 1 ? 2 : value >= 0.01 ? 3 : 4;
	return new Intl.NumberFormat(locale, {
		style: "currency",
		currency: "USD",
		minimumFractionDigits: Math.min(digits, 2),
		maximumFractionDigits: digits,
	}).format(value);
}
