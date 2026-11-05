import { type IBit, IBitTypes } from "../schema/bit/bit";

export const HOSTED_AUDIO_PROVIDERS = [
	{
		value: "hosted:openrouter",
		label: "OpenRouter",
		secret: "OPENROUTER_API_KEY",
	},
	{ value: "hosted:openai", label: "OpenAI", secret: "HOSTED_OPENAI_API_KEY" },
	{ value: "hosted:groq", label: "Groq", secret: "HOSTED_GROQ_API_KEY" },
	{
		value: "hosted:mistral",
		label: "Mistral",
		secret: "HOSTED_MISTRAL_API_KEY",
	},
	{
		value: "hosted:together",
		label: "Together",
		secret: "HOSTED_TOGETHER_API_KEY",
	},
	{ value: "hosted:xai", label: "xAI", secret: "HOSTED_XAI_API_KEY" },
	{ value: "hosted:gemini", label: "Gemini", secret: "HOSTED_GEMINI_API_KEY" },
] as const;

export function isHostedAudioProvider(name: unknown): boolean {
	return (
		typeof name === "string" &&
		HOSTED_AUDIO_PROVIDERS.some(
			(option) => option.value === name.trim().toLowerCase(),
		)
	);
}

function object(value: unknown): Record<string, unknown> {
	return value && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: {};
}

export function isHostedAudio(bit: Pick<IBit, "type" | "parameters">): boolean {
	return (
		[IBitTypes.Stt, IBitTypes.Tts].includes(bit.type) &&
		/^(hosted(?::|$)|premium$|internal$)/i.test(
			String(
				object(object(bit.parameters).provider).provider_name ?? "",
			).trim(),
		)
	);
}

export function createHostedAudioParameters(): Record<string, unknown> {
	return { provider: { provider_name: "hosted:openrouter", model_id: "" } };
}

export function validateHostedAudioParameters(value: unknown): string | null {
	const parameters = object(value);
	const provider = object(parameters.provider);
	if (
		!HOSTED_AUDIO_PROVIDERS.some(
			(option) => option.value === provider.provider_name,
		)
	)
		return "Choose a supported hosted audio provider.";
	const model = provider.model_id;
	if (
		typeof model !== "string" ||
		!model ||
		model.length > 256 ||
		!/^[@A-Za-z0-9/._:-]+$/.test(model) ||
		model.split("/").some((part) => !part || part === "." || part === "..")
	)
		return "Enter the provider's audio model ID, up to 256 characters without spaces or relative paths.";
	if (provider.api_surface != null)
		return "Hosted audio selects its API automatically. Remove the API surface setting.";
	if (
		provider.params != null &&
		(typeof provider.params !== "object" || Array.isArray(provider.params))
	)
		return "Hosted audio settings must contain only an optional access tier.";
	const settings = object(provider.params);
	if (
		Object.entries(settings).some(
			([key, value]) =>
				key !== "tier" || typeof value !== "string" || !value.trim(),
		)
	)
		return "Hosted audio accepts only an access tier. Configure credentials and endpoints on the server.";
	if (
		parameters.default_voice != null &&
		(typeof parameters.default_voice !== "string" ||
			!/^[A-Za-z0-9_.:-]{1,64}$/.test(parameters.default_voice))
	)
		return "Default voice must be 1 to 64 letters, digits, or - _ . :.";
	if (
		parameters.default_language != null &&
		(typeof parameters.default_language !== "string" ||
			!/^[A-Za-z0-9-]{2,16}$/.test(parameters.default_language))
	)
		return "Default language must be a short language code such as en or pt-BR.";
	return null;
}

/** A hosted audio Bit routes through the server and has no local model assets. */
export function updateHostedAudioBit(
	bit: IBit,
	parameters: Record<string, unknown>,
): IBit {
	return {
		...bit,
		download_link: null,
		file_name: null,
		size: 0,
		dependencies: [],
		dependency_tree_hash: "",
		parameters: {
			provider: parameters.provider,
			...(parameters.pricing != null ? { pricing: parameters.pricing } : {}),
			...(bit.type === IBitTypes.Tts && parameters.max_input_characters != null
				? { max_input_characters: parameters.max_input_characters }
				: {}),
			...(bit.type === IBitTypes.Tts && parameters.default_voice != null
				? { default_voice: parameters.default_voice }
				: {}),
			...(bit.type === IBitTypes.Stt && parameters.default_language != null
				? { default_language: parameters.default_language }
				: {}),
		},
	};
}

export function buildHostedAudioBit(bit: IBit): IBit {
	const error = validateHostedAudioParameters(bit.parameters);
	if (error) throw new Error(error);
	return updateHostedAudioBit(bit, object(bit.parameters));
}
