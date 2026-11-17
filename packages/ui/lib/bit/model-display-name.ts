import type { IBit } from "../schema";

/** Providers write their own names — title-casing them reads as a typo. */
const PROVIDER_NAMES: Record<string, string> = {
	"local:stablediffusion": "stable-diffusion.cpp",
	"local:forecast": "Native forecasting",
	anthropic: "Anthropic",
	azure: "Azure OpenAI",
	bedrock: "Amazon Bedrock",
	cohere: "Cohere",
	cloudflare: "Cloudflare Workers AI",
	typesafe: "TypeSafe",
	systemone: "SystemOne endpoint",
	systemone_compatible: "SystemOne endpoint",
	deepseek: "DeepSeek",
	galadriel: "Galadriel",
	gemini: "Google Gemini",
	google: "Google",
	groq: "Groq",
	huggingface: "Hugging Face",
	hyperbolic: "Hyperbolic",
	lmstudio: "LM Studio",
	local: "On-device",
	mlx: "On-device",
	mira: "Mira",
	mistral: "Mistral",
	moonshot: "Moonshot",
	mozilla: "Mozilla",
	ollama: "Ollama",
	openai: "OpenAI",
	openrouter: "OpenRouter",
	perplexity: "Perplexity",
	together: "Together AI",
	vertex: "Vertex AI",
	voyageai: "Voyage AI",
	xai: "xAI",
};

const ON_DEVICE_PROVIDERS: ReadonlySet<string> = new Set(["local", "mlx"]);

/** Display name for a provider key such as `openai` or `local:forecast`. */
export function providerDisplayName(key: string): string {
	return (
		PROVIDER_NAMES[key.toLowerCase()] ??
		key.charAt(0).toUpperCase() + key.slice(1)
	);
}

const HEX_ID_RE = /^[0-9a-f]{16,}$/i;
const UUID_RE =
	/^[0-9a-f]{8}-?[0-9a-f]{4}-?[0-9a-f]{4}-?[0-9a-f]{4}-?[0-9a-f]{12}$/i;
/** cuid2 — the id format every Bit created through the app or hub admin gets. */
const CUID2_RE = /^[a-z][a-z0-9]{23}$/;

/**
 * True for model strings that are an internal Bit / deployment id rather than a
 * name a user would recognise. Providers fall back to the Bit id whenever the
 * model definition carries no explicit `model_id`, which is how these end up in
 * usage stats and trace views.
 */
export function isBitIdLike(value: string): boolean {
	return HEX_ID_RE.test(value) || UUID_RE.test(value) || CUID2_RE.test(value);
}

/** Human-readable name of a model Bit, preferring its catalog metadata. */
export function bitModelName(bit: IBit | undefined | null): string | undefined {
	if (!bit) return undefined;
	const meta = bit.meta?.en ?? Object.values(bit.meta ?? {})[0];
	const candidates = [
		meta?.name,
		bit.model_evaluation?.name,
		bit.model_slug,
		bit.parameters?.provider?.model_id,
	];
	for (const candidate of candidates) {
		const name = typeof candidate === "string" ? candidate.trim() : "";
		if (name && !isBitIdLike(name)) return name;
	}
	return undefined;
}

export interface IModelLabel {
	/** Text to render for this model. */
	label: string;
	/** The label came from a resolved Bit instead of the raw model string. */
	resolved: boolean;
	/** The raw model string is an opaque id that could not be resolved. */
	opaque: boolean;
}

/**
 * Turns a raw model string into something displayable: a resolved Bit name when
 * one is known, the bare model name for provider-qualified strings
 * (`openai/gpt-4o`), or a shortened id as a last resort.
 */
export function modelLabel(
	model: string | undefined | null,
	names?: ReadonlyMap<string, string>,
): IModelLabel {
	const raw = model?.trim() ?? "";
	if (!raw || raw === "unknown") {
		return { label: "Unknown Model", resolved: false, opaque: false };
	}

	const resolved = names?.get(raw);
	if (resolved) return { label: resolved, resolved: true, opaque: false };

	if (isBitIdLike(raw)) {
		return { label: `${raw.slice(0, 8)}…`, resolved: false, opaque: true };
	}

	const parts = raw.split("/");
	return {
		label: parts[parts.length - 1] || raw,
		resolved: false,
		opaque: false,
	};
}

const SNAPSHOT_RE = /[-@](20\d{2})-?(0[1-9]|1[0-2])-?(0[1-9]|[12]\d|3[01])$/;

/** Splits a pinned snapshot date off a model name: `gpt-4o-2024-08-06`. */
function splitSnapshot(name: string): { name: string; snapshot?: string } {
	const match = SNAPSHOT_RE.exec(name);
	const base = match ? name.slice(0, match.index) : "";
	if (!match || !base) return { name };
	return { name: base, snapshot: `${match[1]}-${match[2]}-${match[3]}` };
}

function bitProviderKey(bit: IBit): string | undefined {
	const params = bit.parameters as
		| { provider?: { provider_name?: string } }
		| undefined;
	return (
		params?.provider?.provider_name
			?.replace(/^custom:/i, "")
			.replace(/^hosted:?/i, "")
			.trim()
			.toLowerCase() || undefined
	);
}

function bitProviderModelId(bit: IBit): string | undefined {
	const params = bit.parameters as
		| { provider?: { model_id?: string | null } }
		| undefined;
	const id = params?.provider?.model_id?.trim();
	return id || undefined;
}

function comparableModel(value: string): string {
	const tail = value.trim().split("/").pop() ?? "";
	return splitSnapshot(tail).name.toLowerCase();
}

/**
 * The Bit a reported model string most likely belongs to: its id, or the Bit whose
 * provider `model_id` names the same model once provider prefixes and pinned snapshot
 * dates are set aside (`anthropic/claude-x-20250929` ↔ `claude-x`).
 */
export function matchModelBit(
	model: string | undefined | null,
	bits: readonly IBit[],
): IBit | undefined {
	const raw = model?.trim();
	if (!raw) return undefined;
	const byId = bits.find((bit) => bit.id === raw);
	if (byId) return byId;
	const key = comparableModel(raw);
	if (!key) return undefined;
	const candidates = bits.filter((bit) => {
		const id = bitProviderModelId(bit);
		return id !== undefined && comparableModel(id) === key;
	});
	if (candidates.length < 2) return candidates[0];
	const prefix = raw.includes("/") ? raw.split("/")[0].toLowerCase() : "";
	return (
		candidates.find((bit) => bitProviderModelId(bit) === raw) ??
		candidates.find((bit) => prefix && bitProviderKey(bit) === prefix) ??
		candidates[0]
	);
}

export interface IModelIdentity {
	/** The exact model string the provider reported. */
	raw: string;
	/** Catalog name when a Bit matched, else the model part of `raw`. */
	name: string;
	/** Who served the call, when it is known. */
	provider?: string;
	/** Dated snapshot the provider pinned, as `YYYY-MM-DD`. */
	snapshot?: string;
	/** Served by a runtime on this device. */
	onDevice: boolean;
	/** Named by a Bit from the catalog or the user's profile. */
	known: boolean;
	/** An internal id that no Bit resolved. */
	opaque: boolean;
	/** The call reported no model at all. */
	missing: boolean;
}

/** What a reported model string means to a person: name, provider, snapshot, location. */
export function describeModel(
	model: string | undefined | null,
	bit?: IBit | null,
): IModelIdentity {
	const raw = model?.trim() ?? "";
	const base = { raw, onDevice: false, known: false, opaque: false };
	if (!raw || raw === "unknown") return { ...base, name: "", missing: true };
	const parts = raw.split("/");
	const prefix = parts.length > 1 ? parts[0].toLowerCase() : "";
	const { name, snapshot } = splitSnapshot(parts[parts.length - 1] || raw);
	if (bit) {
		const key = bitProviderKey(bit);
		return {
			...base,
			name: bitModelName(bit) ?? name,
			provider: key ? providerDisplayName(key) : undefined,
			snapshot,
			onDevice: key !== undefined && ON_DEVICE_PROVIDERS.has(key),
			known: true,
			missing: false,
		};
	}
	if (isBitIdLike(raw)) {
		return {
			...base,
			name: `${raw.slice(0, 8)}…`,
			opaque: true,
			missing: false,
		};
	}
	return {
		...base,
		name,
		provider: Object.hasOwn(PROVIDER_NAMES, prefix)
			? PROVIDER_NAMES[prefix]
			: undefined,
		snapshot,
		missing: false,
	};
}
