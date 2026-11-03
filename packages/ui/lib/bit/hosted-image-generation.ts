import { type IBit, IBitTypes } from "../schema/bit/bit";

export const HOSTED_IMAGE_PROVIDERS = [
	{ value: "hosted:cloudflare", label: "Cloudflare Workers AI" },
	{ value: "hosted:openrouter", label: "OpenRouter" },
] as const;

export const HOSTED_CLOUDFLARE_IMAGE_MODELS = [
	{ value: "@cf/black-forest-labs/flux-1-schnell", label: "FLUX.1 Schnell" },
	{
		value: "@cf/stabilityai/stable-diffusion-xl-base-1.0",
		label: "Stable Diffusion XL Base 1.0",
	},
] as const;

function object(value: unknown): Record<string, unknown> {
	return value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: {};
}

export function isHostedImageGeneration(
	bit: Pick<IBit, "type" | "parameters">,
): boolean {
	return (
		bit.type === IBitTypes.ImageGeneration &&
		/^(hosted(?::|$)|premium$|internal$)/i.test(
			String(
				object(object(bit.parameters).provider).provider_name ?? "",
			).trim(),
		)
	);
}

export function createHostedImageParameters(): Record<string, unknown> {
	return {
		provider: {
			provider_name: "hosted:cloudflare",
			model_id: HOSTED_CLOUDFLARE_IMAGE_MODELS[0].value,
		},
		pricing: {},
	};
}

export function createHostedImageBit(bit: IBit): IBit {
	return updateHostedImageBit(
		{
			...bit,
			name: "Hosted image generation",
			model_slug: null,
			authors: [],
			license: "",
			repository: null,
			meta: {
				...bit.meta,
				en: {
					...bit.meta.en,
					name: "Hosted image generation",
					description: "Generate images through a hosted provider.",
					long_description: "",
					docs_url: null,
					website: null,
					tags: ["image-generation", "hosted"],
				},
			},
		},
		createHostedImageParameters(),
	);
}

export function validateHostedImageParameters(value: unknown): string | null {
	const parameters = object(value);
	const provider = object(parameters.provider);
	if (
		!HOSTED_IMAGE_PROVIDERS.some(
			(option) => option.value === provider.provider_name,
		)
	)
		return "Choose Cloudflare Workers AI or OpenRouter for hosted image generation.";
	if (typeof provider.model_id !== "string" || !provider.model_id.trim())
		return "Enter the image model ID expected by the provider.";
	if (
		provider.model_id.length > 256 ||
		!/^[@A-Za-z0-9/._:-]+$/.test(provider.model_id) ||
		provider.model_id
			.split("/")
			.some((segment) => !segment || segment === "." || segment === "..")
	)
		return "Enter a model ID of up to 256 characters using letters, numbers, @, /, periods, underscores, colons or hyphens, without empty or relative path segments.";
	if (
		provider.provider_name === "hosted:cloudflare" &&
		!HOSTED_CLOUDFLARE_IMAGE_MODELS.some(
			(model) => model.value === provider.model_id,
		)
	)
		return "Choose FLUX.1 Schnell or Stable Diffusion XL Base 1.0 for Cloudflare image generation.";
	if (provider.api_surface != null)
		return "Hosted image generation selects its API automatically. Remove the API surface setting.";
	const settings = object(provider.params);
	if (
		provider.params != null &&
		(typeof provider.params !== "object" || Array.isArray(provider.params))
	)
		return "Hosted image provider settings must be a group containing only an optional access tier.";
	if (Object.keys(settings).some((key) => key !== "tier"))
		return "Hosted image provider settings only support an access tier. Configure credentials and endpoints on the server, outside the Bit.";
	if (
		Object.hasOwn(settings, "tier") &&
		(typeof settings.tier !== "string" || !settings.tier.trim())
	)
		return "Access tier must be a nonempty name, or remove it for the server default.";
	const pricing = object(parameters.pricing);
	if (Object.keys(pricing).some((key) => key !== "request_micro_usd"))
		return "Hosted image pricing accepts only a per-image price. Remove token rates and other pricing fields.";
	const price = pricing.request_micro_usd;
	if (typeof price !== "number" || !Number.isSafeInteger(price) || price < 0)
		return "Set a non-negative image price in USD with up to 6 decimal places, no greater than 9007199254.740991. Enter 0 only when the provider cost is zero.";
	return null;
}

export function updateHostedImageTier(
	parameters: Record<string, unknown>,
	tier: string,
): Record<string, unknown> {
	const provider = object(parameters.provider);
	const { tier: _tier, ...settings } = object(provider.params);
	if (tier.trim()) settings.tier = tier.trim();
	return { ...parameters, provider: { ...provider, params: settings } };
}

/** Hosted images have no downloadable weights or local generation settings. */
export function updateHostedImageBit(
	bit: IBit,
	parameters: Record<string, unknown>,
): IBit {
	const { assets: _assets, ...hostedParameters } = parameters;
	return {
		...bit,
		type: IBitTypes.ImageGeneration,
		download_link: null,
		file_name: null,
		size: 0,
		dependencies: [],
		dependency_tree_hash: "",
		parameters: hostedParameters,
	};
}

export function buildHostedImageBit(bit: IBit): IBit {
	const error = validateHostedImageParameters(bit.parameters);
	if (error) throw new Error(error);
	return updateHostedImageBit(bit, object(bit.parameters));
}
