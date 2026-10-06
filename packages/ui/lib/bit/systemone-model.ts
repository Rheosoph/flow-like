import { type IBit, IBitTypes } from "../schema/bit/bit";
import type { ISystemOneParameters } from "../schema/bit/bit/systemone-parameters";

export const SYSTEMONE_HOSTED_PROVIDERS = [
	{ value: "hosted:openrouter", label: "OpenRouter" },
	{ value: "hosted:typesafe", label: "TypeSafe" },
	{ value: "hosted:cloudflare", label: "Cloudflare Workers AI" },
	{
		value: "hosted:systemone_compatible",
		label: "Configured SystemOne endpoint",
	},
] as const;

function object(value: unknown): Record<string, unknown> {
	return value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: {};
}

export const SYSTEMONE_CUSTOM_PROVIDERS = [
	{ value: "custom:systemone", label: "Your SystemOne endpoint" },
	{ value: "custom:typesafe", label: "TypeSafe" },
	{ value: "custom:openrouter", label: "OpenRouter" },
] as const;

export function createSystemOneParameters(): ISystemOneParameters {
	return { context_length: 4096, provider: { provider_name: "Local" } };
}

export function isHostedSystemOne(bit: IBit): boolean {
	return (
		bit.type === IBitTypes.SystemOne &&
		String(bit.parameters?.provider?.provider_name ?? "")
			.toLowerCase()
			.startsWith("hosted:")
	);
}

export function validateSystemOneParameters(
	value: unknown,
	scope: "admin" | "custom" = "admin",
): string | null {
	const parameters = object(value);
	if (
		!Number.isInteger(parameters.context_length) ||
		Number(parameters.context_length) <= 0 ||
		Number(parameters.context_length) > 4_294_967_295
	)
		return "Context length must be a positive whole number up to 4294967295.";
	const provider = object(parameters.provider);
	const settings = object(provider.params);
	const name = String(provider.provider_name ?? "")
		.trim()
		.toLowerCase();
	const hosted = SYSTEMONE_HOSTED_PROVIDERS.some(
		(option) => option.value === name,
	);
	if (
		name !== "local" &&
		!(scope === "custom" && name === "device") &&
		!(scope === "admin" && hosted) &&
		!(
			scope === "custom" &&
			SYSTEMONE_CUSTOM_PROVIDERS.some((option) => option.value === name)
		)
	)
		return "Choose a SystemOne provider. Chat providers cannot answer SystemOne questions.";
	if (
		name === "device" &&
		(!String(settings.device_id ?? "").trim() ||
			!String(settings.model ?? "").trim() ||
			settings.kind !== "systemone")
	)
		return "Select a SystemOne model on a device.";
	if (provider.api_surface != null && provider.api_surface !== "SystemOne")
		return "SystemOne uses its native API. Remove the chat API surface setting.";
	const modelId = provider.model_id ?? settings.model_id;
	if (name !== "local" && (typeof modelId !== "string" || !modelId.trim()))
		return "Enter the SystemOne model ID expected by the provider.";
	if (
		hosted &&
		["endpoint", "base_url", "api_key", "headers", "access_token"].some(
			(key) => settings[key] != null,
		)
	)
		return "Configure hosted endpoints and credentials on the server, outside the Bit.";
	if (
		name === "custom:systemone" ||
		(name.startsWith("custom:") && settings.endpoint)
	) {
		try {
			const url = new URL(String(settings.endpoint ?? ""));
			if (
				!["https:", "http:"].includes(url.protocol) ||
				!url.hostname ||
				url.username ||
				url.password ||
				url.href.includes("?") ||
				url.href.includes("#")
			)
				throw new Error();
		} catch {
			return "Enter an HTTP or HTTPS endpoint URL without credentials, query parameters, or a fragment.";
		}
	}
	return null;
}

export function validateSystemOneBit(
	bit: IBit,
	projection?: IBit,
): string | null {
	const error = validateSystemOneParameters(bit.parameters);
	if (error) return error;
	if (
		!isHostedSystemOne(bit) &&
		(!bit.download_link || !bit.file_name?.toLowerCase().endsWith(".gguf"))
	)
		return "Local SystemOne models need a GGUF download URL and a .gguf file name.";
	if (
		!isHostedSystemOne(bit) &&
		projection &&
		(!projection.download_link ||
			!projection.file_name?.toLowerCase().endsWith(".gguf"))
	)
		return "The optional SystemOne image projector needs a GGUF download URL and a .gguf file name.";
	return null;
}

export function updateSystemOneBit(
	bit: IBit,
	parameters: Record<string, unknown>,
): IBit {
	const next = { ...bit, parameters };
	return isHostedSystemOne(next)
		? {
				...next,
				download_link: null,
				file_name: null,
				size: 0,
				dependencies: [],
			}
		: next;
}
