import type { IBit, IMetadata } from "../../lib/schema/bit/bit";
import { IBitTypes } from "../../lib/schema/bit/bit";
import type { IApiState } from "../../state/backend-state/api-state";
import type { IProfile } from "../../types";

export const SECRET_KEYS = [
	"api_key",
	"service_account_json",
	"access_token",
	"headers",
];
export function record(value: unknown): Record<string, unknown> {
	return value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: {};
}
export function clone<T>(value: T): T {
	return structuredClone(value);
}
export function emptyMetadata(): IMetadata {
	const time = {
		secs_since_epoch: Math.floor(Date.now() / 1000),
		nanos_since_epoch: 0,
	};
	return {
		name: "",
		description: "",
		tags: [],
		preview_media: [],
		created_at: time,
		updated_at: time,
	};
}
export function bitMetadata(bit: IBit): IMetadata {
	return bit.meta?.en ?? Object.values(bit.meta ?? {})[0] ?? emptyMetadata();
}
export function splitBitSecrets(bit: IBit) {
	const copy = clone(bit);
	const params = record(record(record(copy.parameters).provider).params);
	const secrets: Record<string, unknown> = {};
	if (params && typeof params === "object")
		for (const key of SECRET_KEYS) {
			if (key in params) {
				secrets[key] = params[key];
				delete params[key];
			}
		}
	return { bit: copy, secrets };
}
export function coreChanged(before: IBit, after: IBit): boolean {
	const { meta: _oldMeta, updated: _oldUpdated, ...oldCore } = before;
	const { meta: _newMeta, updated: _newUpdated, ...newCore } = after;
	return JSON.stringify(oldCore) !== JSON.stringify(newCore);
}

export const HOSTED_PRICING_FIELDS = [
	{
		key: "input_micro_usd_per_million_tokens",
		label: "USD per 1M input tokens",
		optional: false,
	},
	{
		key: "output_micro_usd_per_million_tokens",
		label: "USD per 1M output tokens",
		optional: false,
	},
	{ key: "request_micro_usd", label: "USD per request", optional: true },
] as const;
export type HostedPricingField = (typeof HOSTED_PRICING_FIELDS)[number]["key"];

export function usdToMicroUsd(text: string): number | null {
	const match = /^(\d*)(?:\.(\d{0,6}))?$/.exec(text.trim());
	if (!match || (!match[1] && !match[2])) return null;
	// Parse the decimal digits directly so floating-point rounding cannot change a rate.
	const micros = Number(`${match[1] || "0"}${(match[2] ?? "").padEnd(6, "0")}`);
	return Number.isSafeInteger(micros) && micros >= 0 ? micros : null;
}

export function microUsdToUsd(value: unknown): string {
	if (value == null) return "";
	if (typeof value !== "number" || !Number.isSafeInteger(value) || value < 0)
		return String(value);
	const digits = String(value).padStart(7, "0");
	const fraction = digits.slice(-6).replace(/0+$/, "");
	return `${digits.slice(0, -6)}${fraction ? `.${fraction}` : ""}`;
}

export function updateBitPricingField(
	parameters: unknown,
	field: HostedPricingField,
	text: string,
): Record<string, unknown> {
	const params = record(parameters);
	const pricing = { ...record(params.pricing) };
	if (!text.trim()) delete pricing[field];
	// Keep invalid input in the draft so validation blocks saving it.
	else pricing[field] = usdToMicroUsd(text) ?? text;
	return { ...params, pricing };
}

export function validateBitPricing(value: unknown): string | null {
	if (value == null) return null;
	if (typeof value !== "object" || Array.isArray(value))
		return "Hosted pricing must be a group of rates, or remove pricing.";
	const pricing = record(value);
	for (const { key, label, optional } of HOSTED_PRICING_FIELDS) {
		const rate = pricing[key];
		if (optional && rate === undefined) continue;
		if (typeof rate !== "number" || !Number.isSafeInteger(rate) || rate < 0)
			return `${label} must be a non-negative amount with up to 6 decimal places, no greater than 9007199254.740991.`;
	}
	return null;
}

export function validateBitDraft(
	bit: IBit,
	scope: "custom" | "admin",
	original?: IBit,
): string | null {
	if (
		Object.values(bit.meta ?? {}).some((meta) => !meta.name.trim()) ||
		!Object.keys(bit.meta ?? {}).length
	)
		return "Give each language a display name before saving.";
	if (scope === "custom" && !bit.meta.en?.name.trim())
		return "Add an English display name before saving.";
	const params = record(bit.parameters);
	if (scope === "admin" && [IBitTypes.Llm, IBitTypes.Vlm].includes(bit.type)) {
		const pricingError = validateBitPricing(params.pricing);
		if (pricingError) return pricingError;
	}
	if (
		[IBitTypes.Llm, IBitTypes.Vlm].includes(bit.type) &&
		(!original ||
			original.type !== bit.type ||
			JSON.stringify(original.parameters) !== JSON.stringify(bit.parameters))
	) {
		if (
			typeof params.context_length !== "number" ||
			!Number.isInteger(params.context_length) ||
			params.context_length <= 0
		)
			return "Context length must be a positive whole number.";
		const providerName = record(params.provider).provider_name;
		if (typeof providerName !== "string" || !providerName.trim())
			return "Enter a provider name in Parameters.";
	}
	const provider = String(
		record(params.provider).provider_name ?? "",
	).toLowerCase();
	if (
		provider === "mlx" &&
		[IBitTypes.Llm, IBitTypes.Vlm].includes(bit.type) &&
		scope === "admin" &&
		!bit.dependencies.length
	)
		return "MLX models need at least one model-file dependency.";
	if (bit.size != null && (!Number.isSafeInteger(bit.size) || bit.size < 0))
		return "File size must be a non-negative whole number.";
	return null;
}

// Checkpoints record completed requests so a retry only sends outstanding changes.
export async function saveAdminBit(
	api: IApiState,
	profile: IProfile,
	original: IBit,
	draft: IBit,
	checkpoint: (bit: IBit) => void = () => {},
) {
	let saved = clone(original);
	if (coreChanged(original, draft)) {
		let finalBit: IBit | undefined;
		let streamError: string | undefined;
		await api.stream<Record<string, unknown>>(
			profile,
			`admin/bit/${encodeURIComponent(original.id)}`,
			{ method: "PUT", body: JSON.stringify(draft) },
			(event) => {
				if (typeof event.error === "string") streamError = event.error;
				if (
					typeof event.message === "string" &&
					!event.id &&
					typeof event.stage !== "string"
				)
					streamError = event.message;
				if (typeof event.id === "string") finalBit = event as unknown as IBit;
			},
		);
		if (streamError || !finalBit)
			throw new Error(
				streamError || "The bit update did not complete. Try saving again.",
			);
		saved = {
			...finalBit,
			meta: finalBit.id === original.id ? saved.meta : {},
		};
		checkpoint(clone(saved));
	}
	for (const [language, metadata] of Object.entries(draft.meta ?? {})) {
		if (
			saved.id === original.id &&
			JSON.stringify(metadata) === JSON.stringify(saved.meta?.[language])
		)
			continue;
		await api.put(
			profile,
			`admin/bit/${encodeURIComponent(saved.id)}/${encodeURIComponent(language)}`,
			metadata,
		);
		saved = { ...saved, meta: { ...saved.meta, [language]: clone(metadata) } };
		checkpoint(clone(saved));
	}
	return saved;
}
