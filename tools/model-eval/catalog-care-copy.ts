export interface CareCopyBit {
	id: string;
	type: string;
	repository?: unknown;
	file_name?: unknown;
	parameters?: unknown;
	meta?: unknown;
}

export interface CatalogCopyPatch {
	bit_id: string;
	name: string;
	description: string;
	long_description: string | null;
	sources: string[];
	reasons: string[];
}

interface CopyOverride {
	model_id: string | null;
	repository: string | null;
	file_name: string | null;
	name?: string;
	description?: string;
	previous_description?: string;
	previous_introduction?: string;
	introduction?: string;
	paragraph_replacements?: Array<{ previous: string; replacement: string }>;
	routing_note?: string;
	sources: string[];
}

const MODEL_TYPES = new Set([
	"Llm",
	"Vlm",
	"Embedding",
	"ImageEmbedding",
	"Tts",
	"Stt",
	"SystemOne",
	"ImageGeneration",
	"VideoGeneration",
	"Forecasting",
]);

function object(value: unknown): Record<string, unknown> {
	return value !== null && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: {};
}

function optionalString(value: unknown): string | null {
	return typeof value === "string" ? value : null;
}

// These reviewed introductions replace only their exact historical text. New notes,
// benchmark evidence and preview attribution appended later remain unchanged.
export function getCatalogCopyPatch(bit: CareCopyBit): CatalogCopyPatch | null {
	if (!MODEL_TYPES.has(bit.type)) return null;
	const override = COPY_OVERRIDES[bit.id];
	if (!override) return null;
	const provider = object(object(bit.parameters).provider);
	if (
		optionalString(provider.model_id) !== override.model_id ||
		optionalString(bit.repository) !== override.repository ||
		optionalString(bit.file_name) !== override.file_name
	) {
		throw new Error(`Catalog copy identity changed for ${bit.id}`);
	}
	const current = object(object(bit.meta).en);
	const oldName = optionalString(current.name) ?? "";
	const oldDescription = optionalString(current.description) ?? "";
	const oldLong = optionalString(current.long_description);
	const name = override.name ?? oldName;
	const description = override.description ?? oldDescription;
	let longDescription = oldLong;
	if (override.introduction !== undefined) {
		const previous = override.previous_introduction ?? "";
		if (
			previous &&
			(oldLong === previous || oldLong?.startsWith(`${previous}\n\n`))
		) {
			longDescription =
				override.introduction + (oldLong?.slice(previous.length) ?? "");
		} else if (!oldLong && !previous) {
			longDescription = override.introduction;
		}
	}
	if (longDescription && override.paragraph_replacements) {
		const replacements = new Map(
			override.paragraph_replacements.map((entry) => [
				entry.previous,
				entry.replacement,
			]),
		);
		longDescription = longDescription
			.split("\n\n")
			.map((paragraph) => replacements.get(paragraph) ?? paragraph)
			.join("\n\n");
	}
	if (override.routing_note) {
		const paragraphs = (longDescription ?? "")
			.split("\n\n")
			.filter(
				(paragraph) => !paragraph.startsWith("Verified preset routing ("),
			);
		longDescription = [
			...paragraphs.filter(Boolean),
			override.routing_note,
		].join("\n\n");
	}
	// Retain detailed runtime and pricing context when a short card description changes.
	// Only the boilerplate provider sentence is removed from card descriptions.
	if (!name || !description)
		throw new Error(`Incomplete catalog copy for ${bit.id}`);
	const reasons: string[] = [];
	if (name !== oldName)
		reasons.push(
			"Use a specific model name without the redundant hosting suffix.",
		);
	if (description !== oldDescription)
		reasons.push(
			"Describe the model's purpose and configuration in plain language.",
		);
	if (longDescription !== oldLong)
		reasons.push(
			"Replace the reviewed legacy introduction while preserving later evidence and notes.",
		);
	if (!reasons.length) return null;
	return {
		bit_id: bit.id,
		name,
		description,
		long_description: longDescription,
		sources: [...override.sources],
		reasons,
	};
}

// Publisher references and OpenRouter catalog identities were checked on 2026-11-16.
const COPY_OVERRIDES: Record<string, CopyOverride> = {
	or0024b06cd282b54e9ff14d: {
		model_id: "qwen/qwen-audio-3.0-tts-plus",
		repository: "https://openrouter.ai/qwen/qwen-audio-3.0-tts-plus",
		file_name: null,
		sources: ["https://openrouter.ai/qwen/qwen-audio-3.0-tts-plus"],
		name: "Qwen-Audio-3.0-TTS Plus",
		description:
			"Generate narration and spoken replies with the Plus tier of Qwen Audio 3.0 TTS.",
		previous_description:
			"Speak replies and narration with the longanlingxin voice. Hosted through OpenRouter.",
	},
	or006774d8b29ba0a02fffb2: {
		model_id: "recraft/recraft-v4.1-utility",
		repository: "https://openrouter.ai/recraft/recraft-v4.1-utility",
		file_name: null,
		sources: ["https://openrouter.ai/recraft/recraft-v4.1-utility"],
		name: "Recraft V4.1 Utility",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	or03cce557187fa4ef4534c2: {
		model_id: "nvidia/nemotron-3.5-asr-streaming-multilingual-0.6b",
		repository:
			"https://openrouter.ai/nvidia/nemotron-3.5-asr-streaming-multilingual-0.6b",
		file_name: null,
		sources: [
			"https://openrouter.ai/nvidia/nemotron-3.5-asr-streaming-multilingual-0.6b",
		],
		name: "NVIDIA Nemotron 3.5 ASR Streaming Multilingual 0.6B",
		description:
			"Transcribe multilingual speech with NVIDIA's compact streaming ASR model.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	or040598c229c7aa7bb611c0: {
		model_id: "black-forest-labs/flux.2-pro",
		repository: "https://openrouter.ai/black-forest-labs/flux.2-pro",
		file_name: null,
		sources: ["https://openrouter.ai/black-forest-labs/flux.2-pro"],
		name: "FLUX.2 Pro",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	or04b6880b575ba8d9ff07e6: {
		model_id: "moondream/parakeet-ultra",
		repository: "https://openrouter.ai/moondream/parakeet-ultra",
		file_name: null,
		sources: ["https://openrouter.ai/moondream/parakeet-ultra"],
		name: "Parakeet Ultra",
		description:
			"Transcribe recordings with Moondream's adapted Parakeet TDT 0.6B v3 model.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	or051f6583ca6197614fc455: {
		model_id: "qwen/qwen3-asr-1.7b",
		repository: "https://openrouter.ai/qwen/qwen3-asr-1.7b",
		file_name: null,
		sources: ["https://openrouter.ai/qwen/qwen3-asr-1.7b"],
		name: "Qwen3 ASR 1.7B",
		description:
			"Transcribe multilingual speech and Chinese dialects with Qwen3 ASR 1.7B.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	or05423a4645ff6e261f3555: {
		model_id: "sourceful/riverflow-v2-fast",
		repository: "https://openrouter.ai/sourceful/riverflow-v2-fast",
		file_name: null,
		sources: ["https://openrouter.ai/sourceful/riverflow-v2-fast"],
		name: "Riverflow V2 Fast",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	or070054c3195f2b6803c9cb: {
		model_id: "perplexity/pplx-embed-v1-4b",
		repository: "https://openrouter.ai/perplexity/pplx-embed-v1-4b",
		file_name: null,
		sources: ["https://openrouter.ai/perplexity/pplx-embed-v1-4b"],
		name: "Perplexity Embed V1 4B",
		description:
			"Embed text in 2560 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 2560 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or0bcf4008a2eb0f4b194502: {
		model_id: "deepgram/flux-tts",
		repository: "https://openrouter.ai/deepgram/flux-tts",
		file_name: null,
		sources: ["https://openrouter.ai/deepgram/flux-tts"],
		name: "Deepgram Flux TTS",
		description:
			"Generate expressive English speech with Deepgram's Flux voices.",
		previous_description:
			"Speak replies and narration with the flux-alexis-en voice. Hosted through OpenRouter.",
	},
	or0be5d6e7b0d348a3015707: {
		model_id: "mistralai/voxtral-mini-tts-3-0",
		repository: "https://openrouter.ai/mistralai/voxtral-mini-tts-3-0",
		file_name: null,
		sources: ["https://openrouter.ai/mistralai/voxtral-mini-tts-3-0"],
		name: "Voxtral TTS 3",
		description:
			"Generate speech with Voxtral TTS 3's preset voices and delivery controls.",
		previous_description:
			"Speak English replies with the en_paul_neutral voice. Hosted through OpenRouter.",
	},
	or0cddd18c31e276889a2cbd: {
		model_id: "canopylabs/orpheus-3b-0.1-ft",
		repository: "https://openrouter.ai/canopylabs/orpheus-3b-0.1-ft",
		file_name: null,
		sources: ["https://openrouter.ai/canopylabs/orpheus-3b-0.1-ft"],
		name: "Orpheus 3B",
		description:
			"Generate English narration and conversational speech with Orpheus 3B's preset voices.",
		previous_description:
			"Speak conversational replies with the tara voice. Hosted through OpenRouter.",
	},
	or12da8833ff023052bdd476: {
		model_id: "fish-audio/s1",
		repository: "https://openrouter.ai/fish-audio/s1",
		file_name: null,
		sources: ["https://openrouter.ai/fish-audio/s1"],
		name: "Fish Audio S1",
		description:
			"Generate multilingual speech with emotion and speaking-style controls.",
		previous_description:
			"Speak replies and narration with the provider default voice. Hosted through OpenRouter.",
	},
	or13e612dce1e72048aaba3f: {
		model_id: "elevenlabs/eleven-turbo-v2.5",
		repository: "https://openrouter.ai/elevenlabs/eleven-turbo-v2.5",
		file_name: null,
		sources: ["https://openrouter.ai/elevenlabs/eleven-turbo-v2.5"],
		name: "Eleven Turbo v2.5",
		description:
			"Generate multilingual speech with Eleven Turbo v2.5 for interactive voice applications.",
		previous_description:
			"Speak replies and narration with the george voice. Hosted through OpenRouter.",
	},
	or18d56b103ce19122194c2a: {
		model_id: "krea/krea-2-medium-turbo",
		repository: "https://openrouter.ai/krea/krea-2-medium-turbo",
		file_name: null,
		sources: ["https://openrouter.ai/krea/krea-2-medium-turbo"],
		name: "Krea 2 Medium Turbo",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	or194d545eff7839460b9c26: {
		model_id: "deepgram/nova-3",
		repository: "https://openrouter.ai/deepgram/nova-3",
		file_name: null,
		sources: ["https://openrouter.ai/deepgram/nova-3"],
		name: "Deepgram Nova-3",
		description:
			"Transcribe monolingual or multilingual recordings with Deepgram Nova-3.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	or1a6fb450ba8c3d5ac9e652: {
		model_id: "fish-audio/s2.1-pro-free:free",
		repository: "https://openrouter.ai/fish-audio/s2.1-pro-free:free",
		file_name: null,
		sources: ["https://openrouter.ai/fish-audio/s2.1-pro-free:free"],
		name: "Fish Audio S2.1 Pro (Free)",
		description:
			"Try Fish Audio S2.1 Pro speech synthesis through its free route, with separate availability and latency limits.",
		previous_description:
			"Speak replies and narration with the provider default voice. Hosted through OpenRouter.",
	},
	or1aea787fcd4302589fefc1: {
		model_id: "openai/whisper-1",
		repository: "https://openrouter.ai/openai/whisper-1",
		file_name: null,
		sources: ["https://openrouter.ai/openai/whisper-1"],
		name: "Whisper 1",
		description:
			"Transcribe uploaded recordings with OpenAI's hosted Whisper API.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	or1cddc44178076b49fb126e: {
		model_id: "elevenlabs/eleven-v3-conversational",
		repository: "https://openrouter.ai/elevenlabs/eleven-v3-conversational",
		file_name: null,
		sources: ["https://openrouter.ai/elevenlabs/eleven-v3-conversational"],
		name: "Eleven v3 Conversational",
		description:
			"Generate multilingual dialogue with the conversational variant of Eleven v3.",
		previous_description:
			"Speak replies and narration with the george voice. Hosted through OpenRouter.",
	},
	or21c3c2bbeb3a3a20d37738: {
		model_id: "recraft/recraft-v4-pro",
		repository: "https://openrouter.ai/recraft/recraft-v4-pro",
		file_name: null,
		sources: ["https://openrouter.ai/recraft/recraft-v4-pro"],
		name: "Recraft V4 Pro",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	or2277c3587c369d7b633ebf: {
		model_id: "elevenlabs/eleven-turbo-v2",
		repository: "https://openrouter.ai/elevenlabs/eleven-turbo-v2",
		file_name: null,
		sources: ["https://openrouter.ai/elevenlabs/eleven-turbo-v2"],
		name: "Eleven Turbo v2",
		description:
			"Generate English speech with Eleven Turbo v2 for interactive voice applications.",
		previous_description:
			"Speak replies and narration with the george voice. Hosted through OpenRouter.",
	},
	or23093aa88c32f6a454a6f8: {
		model_id: "cohere/embed-5-fast",
		repository: "https://openrouter.ai/cohere/embed-5-fast",
		file_name: null,
		sources: ["https://openrouter.ai/cohere/embed-5-fast"],
		name: "Cohere Embed 5 Fast",
		description:
			"Embed text in 2048 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 2048 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or2481947cf775e697ade8d0: {
		model_id: "qwen/qwen-image-3",
		repository: "https://openrouter.ai/qwen/qwen-image-3",
		file_name: null,
		sources: ["https://openrouter.ai/qwen/qwen-image-3"],
		name: "Qwen Image 3",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	or29560f4a4de52cc07ddc58: {
		model_id: "openai/text-embedding-3-large",
		repository: "https://openrouter.ai/openai/text-embedding-3-large",
		file_name: null,
		sources: ["https://openrouter.ai/openai/text-embedding-3-large"],
		name: "Text Embedding 3 Large",
		description:
			"Embed text in 3072 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 3072 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or2a4064c518c227835cde1e: {
		model_id: "mistralai/voxtral-mini-transcribe",
		repository: "https://openrouter.ai/mistralai/voxtral-mini-transcribe",
		file_name: null,
		sources: ["https://openrouter.ai/mistralai/voxtral-mini-transcribe"],
		name: "Voxtral Mini Transcribe",
		description:
			"Transcribe meetings and recordings with Mistral's Voxtral Mini transcription model.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	or2bed8c941b9cb8aaf6e59d: {
		model_id: "assemblyai/universal-3-6-pro",
		repository: "https://openrouter.ai/assemblyai/universal-3-6-pro",
		file_name: null,
		sources: ["https://openrouter.ai/assemblyai/universal-3-6-pro"],
		name: "AssemblyAI Universal-3.6 Pro",
		description:
			"Transcribe recordings with AssemblyAI Universal-3.6 Pro, the successor to Universal-3.5 Pro.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	or2e362599ad76d92323b77f: {
		model_id: "google/gemini-embedding-001",
		repository: "https://openrouter.ai/google/gemini-embedding-001",
		file_name: null,
		sources: ["https://openrouter.ai/google/gemini-embedding-001"],
		name: "Gemini Embedding 001",
		description:
			"Embed text in 3072 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 3072 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or2f647dfcb52073ee6145a3: {
		model_id: "sourceful/riverflow-v2.6-pro",
		repository: "https://openrouter.ai/sourceful/riverflow-v2.6-pro",
		file_name: null,
		sources: ["https://openrouter.ai/sourceful/riverflow-v2.6-pro"],
		name: "Riverflow V2.6 Pro",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	or2ffb13a563da528e824c8c: {
		model_id: "baai/bge-base-en-v1.5",
		repository: "https://openrouter.ai/baai/bge-base-en-v1.5",
		file_name: null,
		sources: ["https://openrouter.ai/baai/bge-base-en-v1.5"],
		name: "bge-base-en-v1.5",
		description:
			"Embed text in 768 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 768 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or31f4da5c2a46f987206a13: {
		model_id: "openai/text-embedding-ada-002",
		repository: "https://openrouter.ai/openai/text-embedding-ada-002",
		file_name: null,
		sources: ["https://openrouter.ai/openai/text-embedding-ada-002"],
		name: "Text Embedding Ada 002",
		description:
			"Embed text in 1536 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 1536 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or330fb82e5aa410a77f5690: {
		model_id: "fish-audio/s2.1-pro",
		repository: "https://openrouter.ai/fish-audio/s2.1-pro",
		file_name: null,
		sources: ["https://openrouter.ai/fish-audio/s2.1-pro"],
		name: "Fish Audio S2.1 Pro",
		description:
			"Generate multilingual narration and dialogue with natural-language delivery controls.",
		previous_description:
			"Speak replies and narration with the provider default voice. Hosted through OpenRouter.",
	},
	or339aaa080500951530a423: {
		model_id: "recraft/recraft-v4",
		repository: "https://openrouter.ai/recraft/recraft-v4",
		file_name: null,
		sources: ["https://openrouter.ai/recraft/recraft-v4"],
		name: "Recraft V4",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	or3869035d9fba2ce31310a2: {
		model_id: "nvidia/nemotron-3.5-transcribe",
		repository: "https://openrouter.ai/nvidia/nemotron-3.5-transcribe",
		file_name: null,
		sources: ["https://openrouter.ai/nvidia/nemotron-3.5-transcribe"],
		name: "NVIDIA Nemotron 3.5 Transcribe",
		description:
			"Transcribe long multilingual recordings with NVIDIA's Nemotron 3.5 model.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	or450c954dc3bfc90c814525: {
		model_id: "google/gemini-embedding-2-preview",
		repository: "https://openrouter.ai/google/gemini-embedding-2-preview",
		file_name: null,
		sources: ["https://openrouter.ai/google/gemini-embedding-2-preview"],
		name: "Gemini Embedding 2 Preview",
		description:
			"Embed text in 3072 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 3072 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or46fbc99aac9b19dcabb02a: {
		model_id: "assemblyai/assemblyai-tts-1",
		repository: "https://openrouter.ai/assemblyai/assemblyai-tts-1",
		file_name: null,
		sources: ["https://openrouter.ai/assemblyai/assemblyai-tts-1"],
		name: "AssemblyAI TTS 1",
		description:
			"Generate speech for voice agents and multilingual narration using AssemblyAI's voice library.",
		previous_description:
			"Speak replies and narration with the aai-audrey voice. Hosted through OpenRouter.",
	},
	or47163903166e26bc233012: {
		model_id: "mongodb/voyage-code-4",
		repository: "https://openrouter.ai/mongodb/voyage-code-4",
		file_name: null,
		sources: ["https://openrouter.ai/mongodb/voyage-code-4"],
		name: "voyage-code-4",
		description:
			"Embed text in 1024 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 1024 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or4b7a72dea168aad5d6fa0e: {
		model_id: "sentence-transformers/all-mpnet-base-v2",
		repository: "https://openrouter.ai/sentence-transformers/all-mpnet-base-v2",
		file_name: null,
		sources: ["https://openrouter.ai/sentence-transformers/all-mpnet-base-v2"],
		name: "all-mpnet-base-v2",
		description:
			"Embed text in 768 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 768 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or523b87193ecc4596d05b5b: {
		model_id: "elevenlabs/eleven-flash-v2",
		repository: "https://openrouter.ai/elevenlabs/eleven-flash-v2",
		file_name: null,
		sources: ["https://openrouter.ai/elevenlabs/eleven-flash-v2"],
		name: "Eleven Flash v2",
		description: "Generate English spoken replies with Eleven Flash v2.",
		previous_description:
			"Speak replies and narration with the george voice. Hosted through OpenRouter.",
	},
	or526b1be82d27375249ae46: {
		model_id: "openai/gpt-transcribe",
		repository: "https://openrouter.ai/openai/gpt-transcribe",
		file_name: null,
		sources: ["https://openrouter.ai/openai/gpt-transcribe"],
		name: "GPT Transcribe",
		description: "Transcribe recorded speech with OpenAI GPT Transcribe.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	or55033582a4ee9437b9d98a: {
		model_id: "recraft/recraft-v4.1-utility-pro",
		repository: "https://openrouter.ai/recraft/recraft-v4.1-utility-pro",
		file_name: null,
		sources: ["https://openrouter.ai/recraft/recraft-v4.1-utility-pro"],
		name: "Recraft V4.1 Utility Pro",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	or58289c22c3acc421c3236c: {
		model_id: "recraft/recraft-v3",
		repository: "https://openrouter.ai/recraft/recraft-v3",
		file_name: null,
		sources: ["https://openrouter.ai/recraft/recraft-v3"],
		name: "Recraft V3",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	or58ec11c3530201e6aed258: {
		model_id: "intfloat/multilingual-e5-large",
		repository: "https://openrouter.ai/intfloat/multilingual-e5-large",
		file_name: null,
		sources: ["https://openrouter.ai/intfloat/multilingual-e5-large"],
		name: "Multilingual-E5-Large",
		description:
			"Embed text in 1024 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 1024 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or5c3d6a816a985a7871acf4: {
		model_id: "krea/krea-2-large",
		repository: "https://openrouter.ai/krea/krea-2-large",
		file_name: null,
		sources: ["https://openrouter.ai/krea/krea-2-large"],
		name: "Krea 2 Large",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	or629544e39e8283227e556a: {
		model_id: "thenlper/gte-large",
		repository: "https://openrouter.ai/thenlper/gte-large",
		file_name: null,
		sources: ["https://openrouter.ai/thenlper/gte-large"],
		name: "GTE-Large",
		description:
			"Embed text in 1024 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 1024 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or62df423bf2386fbbffa0b0: {
		model_id: "black-forest-labs/flux.2-max",
		repository: "https://openrouter.ai/black-forest-labs/flux.2-max",
		file_name: null,
		sources: ["https://openrouter.ai/black-forest-labs/flux.2-max"],
		name: "FLUX.2 Max",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	or6371952256ca8db9132886: {
		model_id: "intfloat/e5-large-v2",
		repository: "https://openrouter.ai/intfloat/e5-large-v2",
		file_name: null,
		sources: ["https://openrouter.ai/intfloat/e5-large-v2"],
		name: "E5-Large-v2",
		description:
			"Embed text in 1024 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 1024 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or643fe556ed4e577f66e444: {
		model_id: "x-ai/grok-voice-tts-1.0",
		repository: "https://openrouter.ai/x-ai/grok-voice-tts-1.0",
		file_name: null,
		sources: ["https://openrouter.ai/x-ai/grok-voice-tts-1.0"],
		name: "Grok Voice TTS 1.0",
		description:
			"Generate multilingual spoken replies and narration with Grok Voice's built-in voices.",
		previous_description:
			"Speak bot replies and narration with the eve voice. Hosted through OpenRouter.",
	},
	or64f43a84af6b3299a73615: {
		model_id: "microsoft/mai-voice-2-flash",
		repository: "https://openrouter.ai/microsoft/mai-voice-2-flash",
		file_name: null,
		sources: ["https://openrouter.ai/microsoft/mai-voice-2-flash"],
		name: "Microsoft MAI-Voice-2-Flash",
		description:
			"Generate multilingual replies with the Flash variant of MAI-Voice-2 for interactive voice applications.",
		previous_description:
			"Speak replies and narration with the en-US-Harper:MAI-Voice-2 voice. Hosted through OpenRouter.",
	},
	or6a6f844fbcbb76d4fb0068: {
		model_id: "mistralai/codestral-embed-2505",
		repository: "https://openrouter.ai/mistralai/codestral-embed-2505",
		file_name: null,
		sources: ["https://openrouter.ai/mistralai/codestral-embed-2505"],
		name: "Codestral Embed 2505",
		description:
			"Embed text in 1536 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 1536 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or6ae53ed8841d36b2f05b89: {
		model_id: "hexgrad/kokoro-82m",
		repository: "https://openrouter.ai/hexgrad/kokoro-82m",
		file_name: null,
		sources: ["https://openrouter.ai/hexgrad/kokoro-82m"],
		name: "Kokoro 82M",
		description:
			"Generate speech with the compact Kokoro-82M model and selectable voices.",
		previous_description:
			"Read short replies and narration with the af_heart voice. Hosted through OpenRouter.",
	},
	or6d5bac3b0366ed93684cd9: {
		model_id: "sentence-transformers/multi-qa-mpnet-base-dot-v1",
		repository:
			"https://openrouter.ai/sentence-transformers/multi-qa-mpnet-base-dot-v1",
		file_name: null,
		sources: [
			"https://openrouter.ai/sentence-transformers/multi-qa-mpnet-base-dot-v1",
		],
		name: "multi-qa-mpnet-base-dot-v1",
		description:
			"Embed text in 768 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 768 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or6e70bedfc7fb6f604c1a4f: {
		model_id: "openai/whisper-large-v3-turbo",
		repository: "https://openrouter.ai/openai/whisper-large-v3-turbo",
		file_name: null,
		sources: ["https://openrouter.ai/openai/whisper-large-v3-turbo"],
		name: "Whisper Large V3 Turbo",
		description:
			"Transcribe multilingual recordings with the Turbo variant of Whisper Large v3.",
		previous_description:
			"Transcribe call utterances, voice notes, and recordings. Hosted through OpenRouter.",
	},
	or6fe2bc8eabd1b4d15d546f: {
		model_id: "google/chirp-3",
		repository: "https://openrouter.ai/google/chirp-3",
		file_name: null,
		sources: ["https://openrouter.ai/google/chirp-3"],
		name: "Chirp 3",
		description:
			"Transcribe multilingual speech with Google's Chirp 3 speech recognition model.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	or71f5022889c695e617a156: {
		model_id: "mistralai/mistral-embed-2312",
		repository: "https://openrouter.ai/mistralai/mistral-embed-2312",
		file_name: null,
		sources: ["https://openrouter.ai/mistralai/mistral-embed-2312"],
		name: "Mistral Embed 2312",
		description:
			"Embed text in 1024 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 1024 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or74227bdd5b77555f986809: {
		model_id: "fish-audio/s2-pro",
		repository: "https://openrouter.ai/fish-audio/s2-pro",
		file_name: null,
		sources: ["https://openrouter.ai/fish-audio/s2-pro"],
		name: "Fish Audio S2 Pro",
		description:
			"Generate expressive multilingual narration and dialogue with speaking-style controls.",
		previous_description:
			"Speak replies and narration with the provider default voice. Hosted through OpenRouter.",
	},
	or74e2749096ba69256dfe00: {
		model_id: "openai/text-embedding-3-small",
		repository: "https://openrouter.ai/openai/text-embedding-3-small",
		file_name: null,
		sources: ["https://openrouter.ai/openai/text-embedding-3-small"],
		name: "Text Embedding 3 Small",
		description:
			"Embed text in 1536 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 1536 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or77de59f84fb31cac929e66: {
		model_id: "google/gemini-embedding-2",
		repository: "https://openrouter.ai/google/gemini-embedding-2",
		file_name: null,
		sources: ["https://openrouter.ai/google/gemini-embedding-2"],
		name: "Gemini Embedding 2",
		description:
			"Embed text in 3072 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 3072 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or7840348b3531c695e5ad96: {
		model_id: "sentence-transformers/paraphrase-minilm-l6-v2",
		repository:
			"https://openrouter.ai/sentence-transformers/paraphrase-minilm-l6-v2",
		file_name: null,
		sources: [
			"https://openrouter.ai/sentence-transformers/paraphrase-minilm-l6-v2",
		],
		name: "paraphrase-MiniLM-L6-v2",
		description:
			"Embed text in 384 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 384 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or78498962ea8e5ec94fd789: {
		model_id: "mongodb/voyage-4-lite",
		repository: "https://openrouter.ai/mongodb/voyage-4-lite",
		file_name: null,
		sources: ["https://openrouter.ai/mongodb/voyage-4-lite"],
		name: "voyage-4-lite",
		description:
			"Embed text in 1024 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 1024 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or7acc96751c7531502d87ce: {
		model_id: "recraft/recraft-v4.1-flash",
		repository: "https://openrouter.ai/recraft/recraft-v4.1-flash",
		file_name: null,
		sources: ["https://openrouter.ai/recraft/recraft-v4.1-flash"],
		name: "Recraft V4.1 Flash",
		description:
			"Generate images from text prompts for drafts and visual exploration.",
		previous_description:
			"Generate images from text prompts for drafts and visual exploration. Hosted through OpenRouter.",
	},
	or7c64e07f669db08de3fa34: {
		model_id: "deepgram/aura-2",
		repository: "https://openrouter.ai/deepgram/aura-2",
		file_name: null,
		sources: ["https://openrouter.ai/deepgram/aura-2"],
		name: "Deepgram Aura-2",
		description:
			"Generate speech using Deepgram's multilingual Aura-2 voice catalog.",
		previous_description:
			"Speak English replies with the aura-2-thalia-en voice. Hosted through OpenRouter.",
	},
	or81c4e355d7ac504abf9d0b: {
		model_id: "fish-audio/transcribe-1-pro",
		repository: "https://openrouter.ai/fish-audio/transcribe-1-pro",
		file_name: null,
		sources: ["https://openrouter.ai/fish-audio/transcribe-1-pro"],
		name: "Fish Audio Transcribe 1 Pro",
		description:
			"Transcribe interviews, meetings, and podcasts with Fish Audio Transcribe 1 Pro.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	or81e5f39716ab88a6dbe4fa: {
		model_id: "mistralai/voxtral-mini-tts-2603",
		repository: "https://openrouter.ai/mistralai/voxtral-mini-tts-2603",
		file_name: null,
		sources: ["https://openrouter.ai/mistralai/voxtral-mini-tts-2603"],
		name: "Voxtral Mini TTS",
		description:
			"Generate multilingual speech with Mistral's Voxtral Mini TTS model.",
		previous_description:
			"Speak replies and narration with the en_paul_neutral voice. Hosted through OpenRouter.",
	},
	or8acf8852e7953d54480a46: {
		model_id: "cohere/embed-5-pro",
		repository: "https://openrouter.ai/cohere/embed-5-pro",
		file_name: null,
		sources: ["https://openrouter.ai/cohere/embed-5-pro"],
		name: "Cohere Embed 5 Pro",
		description:
			"Embed text in 2048 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 2048 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or90eb4026c65490420b12a6: {
		model_id: "baai/bge-large-en-v1.5",
		repository: "https://openrouter.ai/baai/bge-large-en-v1.5",
		file_name: null,
		sources: ["https://openrouter.ai/baai/bge-large-en-v1.5"],
		name: "bge-large-en-v1.5",
		description:
			"Embed text in 1024 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 1024 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	or913b3f4b494919137db407: {
		model_id: "elevenlabs/eleven-v4-turbo",
		repository: "https://openrouter.ai/elevenlabs/eleven-v4-turbo",
		file_name: null,
		sources: ["https://openrouter.ai/elevenlabs/eleven-v4-turbo"],
		name: "Eleven v4 Turbo",
		description:
			"Generate expressive multilingual speech with the Turbo variant of Eleven v4.",
		previous_description:
			"Speak replies and narration with the george voice. Hosted through OpenRouter.",
	},
	or94ae6193a0cc98fc324b19: {
		model_id: "fish-audio/transcribe-1",
		repository: "https://openrouter.ai/fish-audio/transcribe-1",
		file_name: null,
		sources: ["https://openrouter.ai/fish-audio/transcribe-1"],
		name: "Fish Audio Transcribe 1",
		description:
			"Transcribe recordings with automatic language detection using Fish Audio Transcribe 1.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	or95f5de65dbca3a07ffc435: {
		model_id: "microsoft/mai-voice-2",
		repository: "https://openrouter.ai/microsoft/mai-voice-2",
		file_name: null,
		sources: ["https://openrouter.ai/microsoft/mai-voice-2"],
		name: "Microsoft MAI-Voice-2",
		description:
			"Generate multilingual narration and conversational speech with MAI-Voice-2.",
		previous_description:
			"Speak replies and narration with the en-US-Harper:MAI-Voice-2 voice. Hosted through OpenRouter.",
	},
	or961e4f27bf2cdf0acafba7: {
		model_id: "elevenlabs/eleven-v4",
		repository: "https://openrouter.ai/elevenlabs/eleven-v4",
		file_name: null,
		sources: ["https://openrouter.ai/elevenlabs/eleven-v4"],
		name: "Eleven v4",
		description:
			"Generate expressive multilingual speech with Eleven v4 and inline delivery tags.",
		previous_description:
			"Speak replies and narration with the george voice. Hosted through OpenRouter.",
	},
	or9794eca610a95e1bea284d: {
		model_id: "elevenlabs/eleven-v3",
		repository: "https://openrouter.ai/elevenlabs/eleven-v3",
		file_name: null,
		sources: ["https://openrouter.ai/elevenlabs/eleven-v3"],
		name: "Eleven v3",
		description:
			"Generate expressive multilingual speech with Eleven v3 and inline delivery tags.",
		previous_description:
			"Speak replies and narration with the george voice. Hosted through OpenRouter.",
	},
	or9b43c2138a97891db9b410: {
		model_id: "minimax/speech-2.8-hd",
		repository: "https://openrouter.ai/minimax/speech-2.8-hd",
		file_name: null,
		sources: ["https://openrouter.ai/minimax/speech-2.8-hd"],
		name: "MiniMax Speech 2.8 HD",
		description:
			"Generate spoken replies and narration with MiniMax Speech 2.8 HD and a selected MiniMax voice.",
		previous_description:
			"Speak replies and narration with the English_expressive_narrator voice. Hosted through OpenRouter.",
	},
	or9f8a18f58c7c90f5c1410c: {
		model_id: "bytedance-seed/seedream-4.5",
		repository: "https://openrouter.ai/bytedance-seed/seedream-4.5",
		file_name: null,
		sources: ["https://openrouter.ai/bytedance-seed/seedream-4.5"],
		name: "Seedream 4.5",
		description:
			"Generate images from text prompts in square, portrait, or landscape layouts.",
		previous_description:
			"Generate images from text prompts in square, portrait, or landscape layouts. Hosted through OpenRouter.",
	},
	ora04397c69e60027e9e2922: {
		model_id: "microsoft/mai-transcribe-1.5",
		repository: "https://openrouter.ai/microsoft/mai-transcribe-1.5",
		file_name: null,
		sources: ["https://openrouter.ai/microsoft/mai-transcribe-1.5"],
		name: "Microsoft MAI-Transcribe 1.5",
		description:
			"Transcribe multilingual recordings for captions, call transcripts, and subtitles.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	ora07952123a9cecfac66c7a: {
		model_id: "bytedance-seed/seedream-5-0-pro",
		repository: "https://openrouter.ai/bytedance-seed/seedream-5-0-pro",
		file_name: null,
		sources: ["https://openrouter.ai/bytedance-seed/seedream-5-0-pro"],
		name: "Seedream 5.0 Pro",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	ora471d00b047318f2a30378: {
		model_id: "sentence-transformers/all-minilm-l6-v2",
		repository: "https://openrouter.ai/sentence-transformers/all-minilm-l6-v2",
		file_name: null,
		sources: ["https://openrouter.ai/sentence-transformers/all-minilm-l6-v2"],
		name: "all-MiniLM-L6-v2",
		description:
			"Embed text in 384 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 384 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	ora56dead39c2cd8dca48c3c: {
		model_id: "fish-audio/drama-3",
		repository: "https://openrouter.ai/fish-audio/drama-3",
		file_name: null,
		sources: ["https://openrouter.ai/fish-audio/drama-3"],
		name: "Fish Audio Drama 3",
		description:
			"Direct spoken performances with written instructions for emotion and delivery.",
		previous_description:
			"Speak replies and narration with the provider default voice. Hosted through OpenRouter.",
	},
	ora6ce095140d7541d895271: {
		model_id: "mongodb/voyage-4-large",
		repository: "https://openrouter.ai/mongodb/voyage-4-large",
		file_name: null,
		sources: ["https://openrouter.ai/mongodb/voyage-4-large"],
		name: "voyage-4-large",
		description:
			"Embed text in 1024 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 1024 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	ora8ba3ea4797a858c8ea0fa: {
		model_id: "minimax/speech-2.8-turbo",
		repository: "https://openrouter.ai/minimax/speech-2.8-turbo",
		file_name: null,
		sources: ["https://openrouter.ai/minimax/speech-2.8-turbo"],
		name: "MiniMax Speech 2.8 Turbo",
		description:
			"Generate spoken replies and narration with MiniMax Speech 2.8 Turbo and a selected MiniMax voice.",
		previous_description:
			"Speak replies and narration with the English_expressive_narrator voice. Hosted through OpenRouter.",
	},
	ora8dc4e8ccd0744e102ca37: {
		model_id: "baai/bge-m3",
		repository: "https://openrouter.ai/baai/bge-m3",
		file_name: null,
		sources: ["https://openrouter.ai/baai/bge-m3"],
		name: "bge-m3",
		description:
			"Embed text in 1024 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 1024 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	ora91d189620c861df917031: {
		model_id: "mongodb/voyage-4",
		repository: "https://openrouter.ai/mongodb/voyage-4",
		file_name: null,
		sources: ["https://openrouter.ai/mongodb/voyage-4"],
		name: "voyage-4",
		description:
			"Embed text in 1024 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 1024 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	orabd2cfe9e66375067633e7: {
		model_id: "black-forest-labs/flux.2-klein-4b",
		repository: "https://openrouter.ai/black-forest-labs/flux.2-klein-4b",
		file_name: null,
		sources: ["https://openrouter.ai/black-forest-labs/flux.2-klein-4b"],
		name: "FLUX.2 Klein 4B",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	orb01857baeebd6d70ba8934: {
		model_id: "nvidia/nemotron-3-embed-1b:free",
		repository: "https://openrouter.ai/nvidia/nemotron-3-embed-1b:free",
		file_name: null,
		sources: ["https://openrouter.ai/nvidia/nemotron-3-embed-1b:free"],
		name: "NVIDIA Nemotron 3 Embed 1B (free)",
		description:
			"Embed text in 2048 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 2048 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	orb03befda9fb6f1f21a9bff: {
		model_id: "microsoft/mai-voice-2.1",
		repository: "https://openrouter.ai/microsoft/mai-voice-2.1",
		file_name: null,
		sources: ["https://openrouter.ai/microsoft/mai-voice-2.1"],
		name: "Microsoft MAI-Voice-2.1",
		description:
			"Generate multilingual narration and dialogue with MAI-Voice-2.1's voice catalog.",
		previous_description:
			"Speak replies and narration with the en-US-Harper:MAI-Voice-2.1 voice. Hosted through OpenRouter.",
	},
	orb2521edebdfbe71c2738d2: {
		model_id: "perplexity/pplx-embed-v1-0.6b",
		repository: "https://openrouter.ai/perplexity/pplx-embed-v1-0.6b",
		file_name: null,
		sources: ["https://openrouter.ai/perplexity/pplx-embed-v1-0.6b"],
		name: "Perplexity Embed V1 0.6B",
		description:
			"Embed text in 1024 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 1024 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	orb328371ab60658205bcd3d: {
		model_id: "amazon/titan-embed-text-v2",
		repository: "https://openrouter.ai/amazon/titan-embed-text-v2",
		file_name: null,
		sources: ["https://openrouter.ai/amazon/titan-embed-text-v2"],
		name: "Amazon Titan Text Embeddings V2",
		description:
			"Embed text in 1024 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 1024 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	orb533e8a03eab636d53eddb: {
		model_id: "recraft/recraft-v4.1",
		repository: "https://openrouter.ai/recraft/recraft-v4.1",
		file_name: null,
		sources: ["https://openrouter.ai/recraft/recraft-v4.1"],
		name: "Recraft V4.1",
		description: "Generate illustrations and other images from text prompts.",
		previous_description:
			"Generate illustrations and other images from text prompts. Hosted through OpenRouter.",
	},
	orb69db20b5e9f533dad920d: {
		model_id: "meta/muse-voice-transcribe-1.0",
		repository: "https://openrouter.ai/meta/muse-voice-transcribe-1.0",
		file_name: null,
		sources: ["https://openrouter.ai/meta/muse-voice-transcribe-1.0"],
		name: "Muse Voice Transcribe 1.0",
		description:
			"Transcribe spoken requests and recordings with Meta's Muse Voice Transcribe 1.0.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	orb86bbe6eb522f5cc60c38f: {
		model_id: "mistralai/voxtral-mini-3b-2507",
		repository: "https://openrouter.ai/mistralai/voxtral-mini-3b-2507",
		file_name: null,
		sources: ["https://openrouter.ai/mistralai/voxtral-mini-3b-2507"],
		name: "Voxtral Mini 3B 2507",
		description:
			"Transcribe speech with Mistral's 3B Voxtral Mini audio model.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	orb8e311f960b18142b84273: {
		model_id: "mongodb/voyage-multimodal-3.5",
		repository: "https://openrouter.ai/mongodb/voyage-multimodal-3.5",
		file_name: null,
		sources: ["https://openrouter.ai/mongodb/voyage-multimodal-3.5"],
		name: "voyage-multimodal-3.5",
		description:
			"Embed text in 1024 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 1024 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	orb9502fe1c8ab95a08c9af6: {
		model_id: "mistralai/voxtral-small-24b-2507-stt",
		repository: "https://openrouter.ai/mistralai/voxtral-small-24b-2507-stt",
		file_name: null,
		sources: ["https://openrouter.ai/mistralai/voxtral-small-24b-2507-stt"],
		name: "Voxtral Small 24B 2507 STT",
		description: "Transcribe speech with Mistral's 24B Voxtral Small model.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	orba4c3b7129651577bcabf1: {
		model_id: "elevenlabs/eleven-multilingual-v2",
		repository: "https://openrouter.ai/elevenlabs/eleven-multilingual-v2",
		file_name: null,
		sources: ["https://openrouter.ai/elevenlabs/eleven-multilingual-v2"],
		name: "Eleven Multilingual v2",
		description:
			"Generate multilingual narration for voice-overs, audiobooks, and longer passages.",
		previous_description:
			"Speak replies and narration with the george voice. Hosted through OpenRouter.",
	},
	orba69307f6ec6343e45183d: {
		model_id: "amazon/titan-embed-image-v1",
		repository: "https://openrouter.ai/amazon/titan-embed-image-v1",
		file_name: null,
		sources: ["https://openrouter.ai/amazon/titan-embed-image-v1"],
		name: "Amazon Titan Multimodal Embeddings G1",
		description:
			"Embed text in 1024 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 1024 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	orbbaca0ef65b12bc6b0b8c6: {
		model_id: "mistralai/voxtral-mini-transcribe-3-0",
		repository: "https://openrouter.ai/mistralai/voxtral-mini-transcribe-3-0",
		file_name: null,
		sources: ["https://openrouter.ai/mistralai/voxtral-mini-transcribe-3-0"],
		name: "Voxtral Mini Transcribe 3",
		description:
			"Transcribe recordings with Mistral's compact Voxtral Mini Transcribe 3 model.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	orc05353b9684ca8b5c72640: {
		model_id: "sesame/csm-1b",
		repository: "https://openrouter.ai/sesame/csm-1b",
		file_name: null,
		sources: ["https://openrouter.ai/sesame/csm-1b"],
		name: "CSM 1B",
		description:
			"Generate English conversational speech with Sesame's 1B speech model.",
		previous_description:
			"Speak replies and narration with the conversational_a voice. Hosted through OpenRouter.",
	},
	orc08df32b94bf2d68b9d515: {
		model_id: "sourceful/riverflow-v2.5-fast",
		repository: "https://openrouter.ai/sourceful/riverflow-v2.5-fast",
		file_name: null,
		sources: ["https://openrouter.ai/sourceful/riverflow-v2.5-fast"],
		name: "Riverflow V2.5 Fast",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	orc0bb3a5c36ea00b2754952: {
		model_id: "qwen/qwen-image-3-pro",
		repository: "https://openrouter.ai/qwen/qwen-image-3-pro",
		file_name: null,
		sources: ["https://openrouter.ai/qwen/qwen-image-3-pro"],
		name: "Qwen Image 3 Pro",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	orc63ad74e0926b0240db736: {
		model_id: "nvidia/parakeet-tdt-0.6b-v3",
		repository: "https://openrouter.ai/nvidia/parakeet-tdt-0.6b-v3",
		file_name: null,
		sources: ["https://openrouter.ai/nvidia/parakeet-tdt-0.6b-v3"],
		name: "NVIDIA Parakeet TDT 0.6B v3",
		description:
			"Transcribe multilingual recordings with NVIDIA's 0.6B Parakeet model and automatic language detection.",
		previous_description:
			"Turn recorded speech into text for call transcripts and voice input. Hosted through OpenRouter.",
	},
	orca0de367dddbd800a12a89: {
		model_id: "elevenlabs/scribe-v2-medical",
		repository: "https://openrouter.ai/elevenlabs/scribe-v2-medical",
		file_name: null,
		sources: ["https://openrouter.ai/elevenlabs/scribe-v2-medical"],
		name: "Scribe v2 Medical",
		description:
			"Transcribe speech containing clinical and medical terminology with Scribe v2 Medical.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	orca4d1a6dec7a2e70f9e563: {
		model_id: "black-forest-labs/flux-3-image-klein",
		repository: "https://openrouter.ai/black-forest-labs/flux-3-image-klein",
		file_name: null,
		sources: ["https://openrouter.ai/black-forest-labs/flux-3-image-klein"],
		name: "FLUX 3 Image Klein",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	orcbd75cce388ee049cd285b: {
		model_id: "qwen/qwen-image-2.1-pro",
		repository: "https://openrouter.ai/qwen/qwen-image-2.1-pro",
		file_name: null,
		sources: ["https://openrouter.ai/qwen/qwen-image-2.1-pro"],
		name: "Qwen Image 2.1 Pro",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	orcc00b12d95da6b0e3b76d9: {
		model_id: "qwen/qwen-audio-3.0-tts-flash",
		repository: "https://openrouter.ai/qwen/qwen-audio-3.0-tts-flash",
		file_name: null,
		sources: ["https://openrouter.ai/qwen/qwen-audio-3.0-tts-flash"],
		name: "Qwen-Audio-3.0-TTS Flash",
		description:
			"Generate spoken replies with the Flash tier of Qwen Audio 3.0 TTS.",
		previous_description:
			"Speak replies and narration with the loongjohn voice. Hosted through OpenRouter.",
	},
	ord01e5ccee6e9bf390576df: {
		model_id: "microsoft/mai-voice-2.1-flash",
		repository: "https://openrouter.ai/microsoft/mai-voice-2.1-flash",
		file_name: null,
		sources: ["https://openrouter.ai/microsoft/mai-voice-2.1-flash"],
		name: "Microsoft MAI-Voice-2.1-Flash",
		description:
			"Generate multilingual replies with the Flash variant of MAI-Voice-2.1 for interactive voice applications.",
		previous_description:
			"Speak replies and narration with the en-US-Harper:MAI-Voice-2.1-Flash voice. Hosted through OpenRouter.",
	},
	ord433c9e349d04a792b2d0f: {
		model_id: "krea/krea-2-medium",
		repository: "https://openrouter.ai/krea/krea-2-medium",
		file_name: null,
		sources: ["https://openrouter.ai/krea/krea-2-medium"],
		name: "Krea 2 Medium",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	ord4e10969d1bc2182ccbdde: {
		model_id: "bytedance-seed/seedream-5-0-flash",
		repository: "https://openrouter.ai/bytedance-seed/seedream-5-0-flash",
		file_name: null,
		sources: ["https://openrouter.ai/bytedance-seed/seedream-5-0-flash"],
		name: "Seedream 5.0 Flash",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	ord5018b05b146b967b91978: {
		model_id: "sourceful/riverflow-v2-pro",
		repository: "https://openrouter.ai/sourceful/riverflow-v2-pro",
		file_name: null,
		sources: ["https://openrouter.ai/sourceful/riverflow-v2-pro"],
		name: "Riverflow V2 Pro",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	ord72b8343cb9e9c99bb1262: {
		model_id: "black-forest-labs/flux-3-image",
		repository: "https://openrouter.ai/black-forest-labs/flux-3-image",
		file_name: null,
		sources: ["https://openrouter.ai/black-forest-labs/flux-3-image"],
		name: "FLUX 3 Image",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	ord92abce6c8be7762d98042: {
		model_id: "qwen/qwen-image-2.1-turbo",
		repository: "https://openrouter.ai/qwen/qwen-image-2.1-turbo",
		file_name: null,
		sources: ["https://openrouter.ai/qwen/qwen-image-2.1-turbo"],
		name: "Qwen Image 2.1 Turbo",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	ore48bcc3b7893fe805e0da4: {
		model_id: "assemblyai/universal-3-5-pro",
		repository: "https://openrouter.ai/assemblyai/universal-3-5-pro",
		file_name: null,
		sources: ["https://openrouter.ai/assemblyai/universal-3-5-pro"],
		name: "AssemblyAI Universal-3.5 Pro",
		description:
			"Transcribe recordings with AssemblyAI Universal-3.5 Pro's synchronous transcription API.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	ore5736046ff365c2c064089: {
		model_id: "microsoft/mai-transcribe-2",
		repository: "https://openrouter.ai/microsoft/mai-transcribe-2",
		file_name: null,
		sources: ["https://openrouter.ai/microsoft/mai-transcribe-2"],
		name: "Microsoft MAI-Transcribe 2",
		description:
			"Transcribe multilingual recordings and speech that switches between languages with MAI-Transcribe 2.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	ore61036ffce373e5a5655ee: {
		model_id: "bytedance-seed/seedream-5-0-lite",
		repository: "https://openrouter.ai/bytedance-seed/seedream-5-0-lite",
		file_name: null,
		sources: ["https://openrouter.ai/bytedance-seed/seedream-5-0-lite"],
		name: "Seedream 5.0 Lite",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	oreaac159ea48c90cf938872: {
		model_id: "qwen/qwen3-embedding-4b",
		repository: "https://openrouter.ai/qwen/qwen3-embedding-4b",
		file_name: null,
		sources: ["https://openrouter.ai/qwen/qwen3-embedding-4b"],
		name: "Qwen3 Embedding 4B",
		description:
			"Embed text in 2560 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 2560 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	orec317da2a43d21c0e97f68: {
		model_id: "sourceful/riverflow-v2.5-pro",
		repository: "https://openrouter.ai/sourceful/riverflow-v2.5-pro",
		file_name: null,
		sources: ["https://openrouter.ai/sourceful/riverflow-v2.5-pro"],
		name: "Riverflow V2.5 Pro",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	oree50c55909c2827e225f89: {
		model_id: "qwen/qwen3-asr-0.6b",
		repository: "https://openrouter.ai/qwen/qwen3-asr-0.6b",
		file_name: null,
		sources: ["https://openrouter.ai/qwen/qwen3-asr-0.6b"],
		name: "Qwen3 ASR 0.6B",
		description:
			"Transcribe multilingual speech and Chinese dialects with the smaller Qwen3 ASR 0.6B model.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	oree86f6257ea49f75d3bffc: {
		model_id: "openai/whisper-large-v3",
		repository: "https://openrouter.ai/openai/whisper-large-v3",
		file_name: null,
		sources: ["https://openrouter.ai/openai/whisper-large-v3"],
		name: "Whisper Large V3",
		description:
			"Transcribe multilingual recordings with OpenAI Whisper Large v3.",
		previous_description:
			"Transcribe recorded speech into text for workflows and conversations. Hosted through OpenRouter.",
	},
	oree970fc7a3e34f6621089a: {
		model_id: "qwen/qwen3-embedding-8b",
		repository: "https://openrouter.ai/qwen/qwen3-embedding-8b",
		file_name: null,
		sources: ["https://openrouter.ai/qwen/qwen3-embedding-8b"],
		name: "Qwen3 Embedding 8B",
		description:
			"Embed text in 4096 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 4096 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	orf09f1cf367adcc1f579ae2: {
		model_id: "thenlper/gte-base",
		repository: "https://openrouter.ai/thenlper/gte-base",
		file_name: null,
		sources: ["https://openrouter.ai/thenlper/gte-base"],
		name: "GTE-Base",
		description:
			"Embed text in 768 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 768 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	orf328a75d323aab2dac878a: {
		model_id: "elevenlabs/eleven-flash-v2.5",
		repository: "https://openrouter.ai/elevenlabs/eleven-flash-v2.5",
		file_name: null,
		sources: ["https://openrouter.ai/elevenlabs/eleven-flash-v2.5"],
		name: "Eleven Flash v2.5",
		description: "Generate multilingual spoken replies with Eleven Flash v2.5.",
		previous_description:
			"Speak replies and narration with the george voice. Hosted through OpenRouter.",
	},
	orf3427e56f9bbd86003d019: {
		model_id: "sentence-transformers/all-minilm-l12-v2",
		repository: "https://openrouter.ai/sentence-transformers/all-minilm-l12-v2",
		file_name: null,
		sources: ["https://openrouter.ai/sentence-transformers/all-minilm-l12-v2"],
		name: "all-MiniLM-L12-v2",
		description:
			"Embed text in 384 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 384 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	orf4cfd3075019c7fe266000: {
		model_id: "x-ai/grok-imagine-image-2.0",
		repository: "https://openrouter.ai/x-ai/grok-imagine-image-2.0",
		file_name: null,
		sources: ["https://openrouter.ai/x-ai/grok-imagine-image-2.0"],
		name: "Grok Imagine Image 2.0",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	orf4d3ac44edffc574e5cb70: {
		model_id: "recraft/recraft-v4.1-pro",
		repository: "https://openrouter.ai/recraft/recraft-v4.1-pro",
		file_name: null,
		sources: ["https://openrouter.ai/recraft/recraft-v4.1-pro"],
		name: "Recraft V4.1 Pro",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	orf836c7a15c9f1da271cc39: {
		model_id: "black-forest-labs/flux.2-flex",
		repository: "https://openrouter.ai/black-forest-labs/flux.2-flex",
		file_name: null,
		sources: ["https://openrouter.ai/black-forest-labs/flux.2-flex"],
		name: "FLUX.2 Flex",
		description: "Generate images from text prompts.",
		previous_description:
			"Generate images from text prompts. Hosted through OpenRouter.",
	},
	orf8c8865823bb0b0c4eeac5: {
		model_id: "amazon/titan-embed-text-v1",
		repository: "https://openrouter.ai/amazon/titan-embed-text-v1",
		file_name: null,
		sources: ["https://openrouter.ai/amazon/titan-embed-text-v1"],
		name: "Amazon Titan Embeddings G1 - Text",
		description:
			"Embed text in 1536 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 1536 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	orfa135982a5feee27147ecc: {
		model_id: "intfloat/e5-base-v2",
		repository: "https://openrouter.ai/intfloat/e5-base-v2",
		file_name: null,
		sources: ["https://openrouter.ai/intfloat/e5-base-v2"],
		name: "E5-Base-v2",
		description:
			"Embed text in 768 dimensions for semantic search and retrieval.",
		previous_description:
			"Embed text in 768 dimensions for semantic search and retrieval. Hosted through OpenRouter.",
	},
	orfb9a8e0b664610f93482e1: {
		model_id: "elevenlabs/scribe-v2",
		repository: "https://openrouter.ai/elevenlabs/scribe-v2",
		file_name: null,
		sources: ["https://openrouter.ai/elevenlabs/scribe-v2"],
		name: "Scribe v2",
		description:
			"Transcribe multilingual recordings with ElevenLabs Scribe v2.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	orff755f9b5b697d714574e2: {
		model_id: "mistralai/voxtral-small-transcribe-3-0",
		repository: "https://openrouter.ai/mistralai/voxtral-small-transcribe-3-0",
		file_name: null,
		sources: ["https://openrouter.ai/mistralai/voxtral-small-transcribe-3-0"],
		name: "Voxtral Small Transcribe 3",
		description:
			"Transcribe recordings with Mistral's larger Voxtral Small Transcribe 3 model.",
		previous_description:
			"Transcribe spoken audio into text. Hosted through OpenRouter.",
	},
	adrs5ngk9pky9dnuxfkab9gr: {
		model_id: null,
		repository: "https://huggingface.co/nex-agi/Nex-N2.5-mini",
		file_name: "Nex-N2.5-mini.Q4_K_S.gguf",
		sources: ["https://huggingface.co/nex-agi/Nex-N2.5-mini"],
		name: "Nex-N2.5 Mini (Q4_K_S)",
		description:
			"Use screenshots and text in local workflows that browse, operate a desktop, or use a terminal.",
		previous_description:
			"Built for agents that click, browse and use a terminal.",
		previous_introduction:
			"- Its authors report it operating a desktop and browsing the web at a level normally seen only from hosted services.\n- Only a fraction of its weights run per word, so it answers far faster than its size suggests.\n- Understands screenshots, which is what makes computer use possible.\n- The largest download here at around 20 GB.",
		introduction:
			"Use screenshots and text in local workflows that browse, operate a desktop, or use a terminal.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 262,144 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings.\n\nModel reference: https://huggingface.co/nex-agi/Nex-N2.5-mini",
	},
	byzz4k0sbs9lph8lt1po7vqt: {
		model_id: null,
		repository: "https://huggingface.co/Qwen/Qwen3.8-27B",
		file_name: "Qwen3.8-27B-UD-Q4_K_M.gguf",
		sources: ["https://huggingface.co/Qwen/Qwen3.8-27B"],
		name: "Qwen3.8 27B (UD-Q4_K_M)",
		description: "Work with text and images using the local 27B Qwen3.8 model.",
		previous_description:
			"The strongest open model you can run at home. Reads images as well as text.",
		previous_introduction:
			"- Currently the most downloaded open model anywhere, and it shows: it reasons close to what hosted services managed a year ago.\n- Sees images and video frames natively, so it can read a screenshot or a scanned page.\n- Handles very long documents, up to about 262,000 words of context.\n- Needs roughly 16 GB of free memory, so it wants a well-specified machine.",
		introduction:
			"Work with text and images using the local 27B Qwen3.8 model.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 262,144 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings.\n\nModel reference: https://huggingface.co/Qwen/Qwen3.8-27B",
	},
	c4n20wmhu4wkn9hv690y39yx: {
		model_id: null,
		repository: "https://huggingface.co/ornith-ai/Ornith-1.5-35B-A3B",
		file_name: "Tiel-Coder-35B-A3B-UD-IQ4_XS.gguf",
		sources: [
			"https://api.flow-like.com/api/v1/bit/c4n20wmhu4wkn9hv690y39yx?language=en",
			"https://huggingface.co/ornith-ai/Ornith-1.5-35B-A3B",
		],
		name: "Tiel Coder 35B A3B (UD-IQ4_XS)",
		description: "Generate and review code with the local Tiel Coder model.",
		previous_description:
			"A coding specialist that fixes real bugs in real repositories.",
		previous_introduction:
			"- On a live software-repair benchmark it matched a leading hosted model at medium effort.\n- Tuned for long agentic sessions rather than one-shot answers.\n- Fast for its size, because only about three billion weights run per word.\n- Weaker than general models on exam-style knowledge questions. That is the trade.",
		introduction:
			"Generate and review code with the local Tiel Coder model.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 262,144 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings.\n\nThe download is labeled Tiel Coder, while its catalog link names Ornith 1.5. The linked model's capabilities and benchmarks have not been verified for this download.",
	},
	g30u9tofov9c0cr7qvv759r0: {
		model_id: null,
		repository: "https://huggingface.co/unsloth/gemma-4-26B-A4B-it-GGUF",
		file_name: "gemma-4-26B-A4B-it-UD-Q4_K_M.gguf",
		sources: ["https://huggingface.co/unsloth/gemma-4-26B-A4B-it-GGUF"],
		name: "Gemma 4 26B A4B (UD-Q4_K_M)",
		description:
			"Work with text and images using Gemma 4's 26B mixture-of-experts model.",
		previous_description:
			"A fast, high-performance AI that only activates what it needs — ideal for complex tasks on workstations.",
		previous_introduction:
			"Gemma 4 26B uses a Mixture-of-Experts architecture, meaning it has 26 billion parameters in total but only activates around 4 billion at a time. This makes it exceptionally fast for its intelligence level — you get the reasoning quality of a much larger model at a fraction of the speed and memory cost. It supports text and image input with a 256,000-token context window, making it well-suited for analyzing lengthy documents, multi-step workflows, and coding tasks. Recommended if you have a workstation or a gaming PC with a dedicated GPU and want the best speed-to-quality ratio.",
		introduction:
			"Work with text and images using Gemma 4's 26B mixture-of-experts model.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 262,144 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings.\n\nModel reference: https://huggingface.co/unsloth/gemma-4-26B-A4B-it-GGUF",
	},
	gy8thxkr9oqtl7ztxgst513y: {
		model_id: null,
		repository: "https://huggingface.co/Qwen/Qwen3.5-9B",
		file_name: "Qwen3.5-9B-UD-Q4_K_XL.gguf",
		sources: ["https://huggingface.co/Qwen/Qwen3.5-9B"],
		name: "Qwen3.5 9B (UD-Q4_K_XL)",
		description: "Work with text and images using the local 9B Qwen3.5 model.",
		previous_description:
			"A dependable middle option that reads images and fits most laptops.",
		previous_introduction:
			"- Sits between the small and large models on quality without asking much of your machine.\n- Understands images as well as text.\n- Around 6 GB of memory, and long context up to about 262,000 words.\n- A newer generation exists at 27B if you have the memory for it.",
		introduction:
			"Work with text and images using the local 9B Qwen3.5 model.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 262,144 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings.\n\nModel reference: https://huggingface.co/Qwen/Qwen3.5-9B",
	},
	hmujomb1n03k69znccb7aqup: {
		model_id: null,
		repository: "https://huggingface.co/Qwen/Qwen3.6-27B",
		file_name: "Ternary-Bonsai-27B-Q2_g64.gguf",
		sources: ["https://huggingface.co/prism-ml/Ternary-Bonsai-27B-gguf"],
		name: "Ternary Bonsai 27B (Q2_g64)",
		description:
			"Use the compressed Ternary Bonsai 27B model for local text and image workflows.",
		previous_description: "Large-model quality squeezed into a small download.",
		previous_introduction:
			"- A 27-billion-parameter model compressed to under 8 GB while keeping most of its ability.\n- The best quality-per-gigabyte option in the catalogue by a wide margin.\n- Reads images as well as text.\n- Needs a recent llama.cpp build; older runtimes cannot open its compressed weights.",
		introduction:
			"Use the compressed Ternary Bonsai 27B model for local text and image workflows.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 262,144 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings. Its compressed weights require a recent compatible llama.cpp runtime.\n\nModel reference: https://huggingface.co/prism-ml/Ternary-Bonsai-27B-gguf",
	},
	i6a7iln27vu1t7fir8lk23xz: {
		model_id: null,
		repository: "https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF",
		file_name: "gemma-4-E2B-it-Q4_K_M.gguf",
		sources: ["https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF"],
		name: "Gemma 4 E2B (Q4_K_M)",
		description: "Use Gemma 4 E2B for local text and image tasks.",
		previous_description:
			"A tiny but capable AI that runs on phones and low-end devices — understands text, images, and audio.",
		previous_introduction:
			"Gemma 4 E2B is the smallest model in the Gemma 4 family, designed to run on smartphones and devices with as little as 4 GB of memory. Despite its compact size, it can read and understand text, analyze images, and even process audio like voice recordings. It supports a context window of up to 128,000 tokens, meaning it can work with long documents and conversations. E2B is a great starting point if you want AI capabilities on the go without needing powerful hardware.",
		introduction:
			"Use Gemma 4 E2B for local text and image tasks.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 131,072 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings.\n\nModel reference: https://huggingface.co/unsloth/gemma-4-E2B-it-GGUF",
	},
	k5gx5es5vmdkjp932kovujv0: {
		model_id: null,
		repository: "https://huggingface.co/openbmb/MiniCPM5-2B",
		file_name: "MiniCPM5-2B-Q8_0.gguf",
		sources: ["https://huggingface.co/openbmb/MiniCPM5-2B"],
		name: "MiniCPM5 2B (Q8_0)",
		description: "Generate and revise text with the local 2B MiniCPM5 model.",
		previous_description:
			"Punches far above its size. The best small model we ship.",
		previous_introduction:
			"- Beats every model in our catalogue under 26 billion parameters, at a fraction of the size.\n- Runs in about 1.5 GB, so it is comfortable on a laptop or a phone-class device.\n- Text only. It cannot look at images.\n- Handles around 131,000 words of context.",
		introduction:
			"Generate and revise text with the local 2B MiniCPM5 model.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 131,072 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings.\n\nModel reference: https://huggingface.co/openbmb/MiniCPM5-2B",
	},
	m3mnsg6v81uvikfmr6nssurl: {
		model_id: null,
		repository: "https://huggingface.co/Qwen/Qwen3.8-27B",
		file_name: "Qwen3.8-27B-Uncensored-Q4_K_M.gguf",
		sources: ["https://huggingface.co/Qwen/Qwen3.8-27B"],
		name: "Qwen3.8 27B Uncensored (Q4_K_M)",
		description:
			"A modified local Qwen3.8 27B build for text and image workflows, distributed as Uncensored.",
		previous_description:
			"The same model with its refusals removed. For work the standard build declines.",
		previous_introduction:
			"- Based on the standard model, with image input support. Its intelligence estimate uses the base model as a proxy; this modified build has no direct AA v5.0.1 measurement.\n- The safety training has been stripped out, so it will attempt almost anything you ask.\n- You are responsible for what it produces. Do not put this in front of the public.\n- Needs roughly 17 GB of free memory.",
		introduction:
			"A modified local Qwen3.8 27B build for text and image workflows, distributed as Uncensored.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 262,144 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings. The intelligence estimate uses the base model as a proxy; this modified build has no direct AA v5.0.1 measurement. The label does not establish its refusal behavior or safety performance.\n\nModel reference: https://huggingface.co/Qwen/Qwen3.8-27B",
	},
	mpweiirtyhdu30o3tu008vyo: {
		model_id: null,
		repository: "https://huggingface.co/openbmb/MiniCPM-o-2_6-gguf",
		file_name: "MiniCPM-o-2_6.gguf",
		sources: ["https://huggingface.co/openbmb/MiniCPM-o-2_6-gguf"],
		name: "MiniCPM-o 2.6",
		description:
			"Ask questions about images and extract visible text with the local MiniCPM-o 2.6 model.",
		previous_description:
			"🔥 Leading Visual Capability & 💪 Strong OCR Capability",
		previous_introduction:
			"MiniCPM-o 2.6 is the latest and most capable model in the MiniCPM-o series. The model is built in an end-to-end fashion based on SigLip-400M, Whisper-medium-300M, ChatTTS-200M, and Qwen2.5-7B with a total of 8B parameters. It exhibits a significant performance improvement over MiniCPM-V 2.6, and introduces new features for real-time speech conversation and multimodal live streaming.",
		introduction:
			"Ask questions about images and extract visible text with the local MiniCPM-o 2.6 model.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 32,768 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings.\n\nModel reference: https://huggingface.co/openbmb/MiniCPM-o-2_6-gguf",
	},
	ngbc8xvkrv5mtvcjq81w9n6v: {
		model_id: null,
		repository: "https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507",
		file_name: "Qwen3-4B-Instruct-2507-UD-Q5_K_XL.gguf",
		sources: ["https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507"],
		name: "Qwen3 4B Instruct 2507 (UD-Q5_K_XL)",
		description:
			"Use the July 2025 instruction-tuned Qwen3 4B model for local text tasks. This release uses non-thinking responses.",
		previous_description:
			"Qwen 3 is the latest open-source language model from Alibaba’s Qwen family, available in a wide range of sizes—from compact versions to a powerful 235‑billion‑parameter model. It features a clever “thinking mode” for deeper reasoning and a faster “non‑thinking mode” for everyday queries, handles long conversations, and understands 100+ languages—all under the friendly Apache 2.0 license.\n",
		previous_introduction:
			"Qwen 3 (released in April 2025) represents Alibaba’s most advanced open-weight large language model suite, available under the Apache 2.0 license.\nThe model lineup spans from smaller dense versions (0.6B to 32B parameters) to powerful Mixture-of-Experts (MoE) versions, including the flagship Qwen3‑235B‑A22B (235 b param with 22 b active), and a leaner Qwen3‑30B‑A3B.\nWhat sets Qwen 3 apart is its adaptive “thinking mode”, which enables deep logical reasoning, math, and coding when needed, and a “non‑thinking mode” that delivers quick, general responses—both managed within the same model by toggling via the tokenizer or a “thinking budget” mechanism. Developed using a three-stage pretraining pipeline on an enormous 36 trillion-token multilingual dataset (covering 119 languages and dialects), Qwen 3 excels in reasoning, programming, and multilingual tasks thanks to improved training strategies and architectural refinements.\n\nEmpirical evaluations reveal that Qwen 3 matches or surpasses state-of-the-art models like DeepSeek‑R1, Gemini‑2.5‑Pro, and Grok‑3 in benchmarks for math, code generation, and overall reasoning performance. With its flexibility, efficiency, and license-friendly open-source nature, Qwen 3 offers accessible, high-level AI capabilities for developers, researchers, and organizations worldwide.",
		introduction:
			"Use the July 2025 instruction-tuned Qwen3 4B model for local text tasks. This release uses non-thinking responses.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 262,144 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings. This Bit uses Qwen3-4B-Instruct-2507, rather than the original Qwen3 release with selectable thinking modes.\n\nModel reference: https://huggingface.co/Qwen/Qwen3-4B-Instruct-2507",
	},
	nu6s751b9au32iwpch1namca: {
		model_id: null,
		repository: "https://huggingface.co/XHToken/Spark-X2.5-4B",
		file_name: "Spark-X2.5-4B-Q4_K_M.gguf",
		sources: ["https://huggingface.co/XHToken/Spark-X2.5-4B"],
		name: "Spark X2.5 4B (Q4_K_M)",
		description:
			"Work with long text inputs using the local 4B Spark X2.5 model.",
		previous_description:
			"Reads book-length input and speaks over two hundred languages.",
		previous_introduction:
			"- Takes about a million words of context, several times anything else in the catalogue.\n- Broad language coverage, well past the usual handful.\n- Small and quick at around 2.5 GB.\n- Text only, and less capable than the larger models on difficult reasoning.",
		introduction:
			"Work with long text inputs using the local 4B Spark X2.5 model.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 1,048,576 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings.\n\nModel reference: https://huggingface.co/XHToken/Spark-X2.5-4B",
	},
	o1bygitpw14myg943sh4vjgz: {
		model_id: null,
		repository: "https://huggingface.co/inclusionAI/Ling-3.0-tiny",
		file_name: "Ling-3.0-tiny-Q4_K_M.gguf",
		sources: ["https://huggingface.co/inclusionAI/Ling-3.0-tiny"],
		name: "Ling 3.0 Tiny (Q4_K_M)",
		description:
			"Generate text with the local Ling 3.0 Tiny mixture-of-experts model.",
		previous_description:
			"The fastest model here. Answers almost instantly on a laptop.",
		previous_introduction:
			"- Barely over a billion weights run per word, so it streams text very quickly.\n- Around 5 GB on disk while behaving like a much smaller model at run time.\n- Text only.\n- Choose it for speed and responsiveness rather than for hard problems.",
		introduction:
			"Generate text with the local Ling 3.0 Tiny mixture-of-experts model.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 131,072 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings.\n\nModel reference: https://huggingface.co/inclusionAI/Ling-3.0-tiny",
	},
	qcokoej6wo2t12cgn9yzkfgl: {
		model_id: null,
		repository: "https://huggingface.co/TokenRhythm/NeoHorse-1-9B",
		file_name: "NeoHorse-1-9B-Q4_K_M.gguf",
		sources: ["https://huggingface.co/TokenRhythm/NeoHorse-1-9B"],
		name: "NeoHorse 1 9B (Q4_K_M)",
		description:
			"Follow instructions and use tools with the local NeoHorse 1 9B text model.",
		previous_description:
			"A mid-size model retuned to follow instructions and use tools reliably.",
		previous_introduction:
			"- Measurably better than the model it is built on at following instructions and calling tools.\n- A good fit for automations where predictability matters more than raw intelligence.\n- Around 5 GB, comfortable on most laptops.\n- Text only. The image support of its base model was removed.",
		introduction:
			"Follow instructions and use tools with the local NeoHorse 1 9B text model.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 262,144 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings.\n\nModel reference: https://huggingface.co/TokenRhythm/NeoHorse-1-9B",
	},
	suqz8alvuhzm2vnp9uhpxya8: {
		model_id: null,
		repository: "https://huggingface.co/unsloth/gemma-4-E4B-it-GGUF",
		file_name: "gemma-4-E4B-it-Q4_K_M.gguf",
		sources: ["https://huggingface.co/unsloth/gemma-4-E4B-it-GGUF"],
		name: "Gemma 4 E4B (Q4_K_M)",
		description: "Use Gemma 4 E4B for local text and image tasks.",
		previous_description:
			"A lightweight AI for laptops and tablets — handles text, images, and audio with strong reasoning for its size.",
		previous_introduction:
			"Gemma 4 E4B is a step up from the E2B, offering noticeably better reasoning and understanding while still running on everyday hardware like laptops and tablets. It handles text, images, and audio, making it a versatile choice for tasks like summarizing documents, describing photos, or transcribing speech. With a 128,000-token context window and built-in tool use, E4B hits a sweet spot between capability and efficiency — powerful enough for most everyday tasks, light enough to run locally without a dedicated graphics card.",
		introduction:
			"Use Gemma 4 E4B for local text and image tasks.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 131,072 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings.\n\nModel reference: https://huggingface.co/unsloth/gemma-4-E4B-it-GGUF",
	},
	ur5rilpnhqf1hgc6h1cw0ntw: {
		model_id: null,
		repository: "https://huggingface.co/ibm-granite/granite-4.2-8b",
		file_name: "granite-4.2-8b-Q4_K_M.gguf",
		sources: ["https://huggingface.co/ibm-granite/granite-4.2-8b"],
		name: "Granite 4.2 8B (Q4_K_M)",
		description:
			"Generate and analyze text locally with IBM's Granite 4.2 8B model.",
		previous_description:
			"IBM's business-focused model, with the licence paperwork already sorted.",
		previous_introduction:
			"- Apache licensed and published by IBM, which matters when procurement asks.\n- Solid general-purpose quality at around 5 GB.\n- Text only, no image understanding.\n- Trained with enterprise use in mind rather than chat or creative writing.",
		introduction:
			"Generate and analyze text locally with IBM's Granite 4.2 8B model.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 131,072 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings.\n\nModel reference: https://huggingface.co/ibm-granite/granite-4.2-8b",
	},
	vfrvv768xcetv2p3l4rg22zo: {
		model_id: null,
		repository: "https://huggingface.co/ornith-ai/Ornith-1.5-35B-A3B",
		file_name: "Ornith-1.5-35B-A3B-IQ4_XS.gguf",
		sources: ["https://huggingface.co/ornith-ai/Ornith-1.5-35B-A3B"],
		name: "Ornith 1.5 35B A3B (IQ4_XS)",
		description:
			"Work with text and images using the local Ornith 1.5 mixture-of-experts model.",
		previous_description:
			"The best reasoning we ship for local use, and quick despite its size.",
		previous_introduction:
			"- Reasons at a level that was hosted-only until recently, especially on science and maths.\n- A mixture-of-experts design means only about three billion weights run per word, so it stays responsive.\n- Reads images as well as text.\n- Around 18 GB on disk, and it wants memory to match.",
		introduction:
			"Work with text and images using the local Ornith 1.5 mixture-of-experts model.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 262,144 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings.\n\nModel reference: https://huggingface.co/ornith-ai/Ornith-1.5-35B-A3B",
	},
	vtsrpaaozrs8yuuac8punyvt: {
		model_id: null,
		repository: "https://huggingface.co/Qwen/Qwen3.5-4B",
		file_name: "Qwen3.5-4B-UD-Q4_K_XL.gguf",
		sources: ["https://huggingface.co/Qwen/Qwen3.5-4B"],
		name: "Qwen3.5 4B (UD-Q4_K_XL)",
		description: "Work with text and images using the local 4B Qwen3.5 model.",
		previous_description: "Small, quick and still able to look at images.",
		previous_introduction:
			"- About 3 GB, so it starts fast and leaves room for everything else.\n- Understands images, which is unusual at this size.\n- Long context up to about 262,000 words.\n- Clearly behind the bigger models on hard reasoning, as you would expect.",
		introduction:
			"Work with text and images using the local 4B Qwen3.5 model.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 262,144 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings.\n\nModel reference: https://huggingface.co/Qwen/Qwen3.5-4B",
	},
	wjjp0qmjr1y6lvglqn7h8lw3: {
		model_id: null,
		repository: "https://huggingface.co/unsloth/gemma-4-31B-it-GGUF",
		file_name: "gemma-4-31B-it-Q4_K_M.gguf",
		sources: ["https://huggingface.co/unsloth/gemma-4-31B-it-GGUF"],
		name: "Gemma 4 31B (Q4_K_M)",
		description: "Work with text and images using Gemma 4's 31B dense model.",
		previous_description:
			"The most capable Gemma 4 model — top-tier reasoning and quality for demanding tasks.",
		previous_introduction:
			"Gemma 4 31B is the flagship of the Gemma 4 family and its most powerful model. All 31 billion parameters are active during every response, delivering the highest quality reasoning, writing, coding, and image understanding available in the lineup. It supports a 256,000-token context window, handles complex multi-step problems, and is an excellent foundation for specialized fine-tuning. It requires more powerful hardware — typically a dedicated GPU with 24 GB or more of memory — but in return delivers quality that competes with models many times its size.",
		introduction:
			"Work with text and images using Gemma 4's 31B dense model.\n\nThis Bit loads a GGUF model on your device. The catalog context limit is 262,144 tokens. Device or runtime settings can lower this limit. Memory use depends on the model weights, context length and hardware settings.\n\nModel reference: https://huggingface.co/unsloth/gemma-4-31B-it-GGUF",
	},
	aimtabmsss3o3qh4ntr9i1zo: {
		model_id: "prithivMLmods/WeVisDoc-2B-GGUF",
		repository: "https://huggingface.co/prithivMLmods/WeVisDoc-2B-GGUF",
		file_name: "WeVisDoc-2B.Q4_K_M.gguf",
		sources: ["https://huggingface.co/prithivMLmods/WeVisDoc-2B-GGUF"],
		name: "WeVisDoc 2B Document Reader (Q4_K_M)",
	},
	sg047vxajuytywx532252p3c: {
		model_id: "prithivMLmods/WeVisDoc-4B-GGUF",
		repository: "https://huggingface.co/prithivMLmods/WeVisDoc-4B-GGUF",
		file_name: "WeVisDoc-4B.Q4_K_M.gguf",
		sources: ["https://huggingface.co/prithivMLmods/WeVisDoc-4B-GGUF"],
		name: "WeVisDoc 4B Document Reader (Q4_K_M)",
	},
	jirno2jf6moakajtttdmnqqz: {
		model_id: "jialinyyzz/humanizer",
		repository: "https://huggingface.co/jialinyyzz/humanizer",
		file_name: "humanizer-12b-Q4_K_M.gguf",
		sources: ["https://huggingface.co/jialinyyzz/humanizer"],
		name: "Humanizer 12B (Q4_K_M)",
	},
	jmtmmfse7xfd6t2m9kqmhzh8: {
		model_id: "orcarouter/OrcaSAQ-2-Cyber-27B-Uncensored-GGUF",
		repository:
			"https://huggingface.co/orcarouter/OrcaSAQ-2-Cyber-27B-Uncensored-GGUF",
		file_name: "OrcaSAQ-2-27B-Uncensored.gguf",
		sources: [
			"https://huggingface.co/orcarouter/OrcaSAQ-2-Cyber-27B-Uncensored-GGUF",
		],
		name: "OrcaSAQ 2 Cyber 27B Uncensored",
	},
	taoayv5fghsraocinxsv0334: {
		model_id: "Venastine-Research/Xing4.0-29B-A4B-GGUF",
		repository:
			"https://huggingface.co/Venastine-Research/Xing4.0-29B-A4B-GGUF",
		file_name: "Xing4.0-29B-A4B-Q4_K_M.gguf",
		sources: ["https://huggingface.co/Venastine-Research/Xing4.0-29B-A4B-GGUF"],
		name: "Xing 4.0 29B A4B (Q4_K_M)",
	},
	ypb23js6sng4k7vib2jdl7qa: {
		model_id: "ggml-org/gpt-oss-20b-GGUF",
		repository: "https://huggingface.co/ggml-org/gpt-oss-20b-GGUF",
		file_name: "gpt-oss-20b-MXFP4.gguf",
		sources: ["https://huggingface.co/ggml-org/gpt-oss-20b-GGUF"],
		name: "GPT-OSS 20B (MXFP4)",
	},
	brza8lvz5me5877uu0rnukuz: {
		model_id: null,
		repository: "https://huggingface.co/google/embeddinggemma-300m",
		file_name: "model_q4.onnx",
		sources: ["https://huggingface.co/google/embeddinggemma-300m"],
		name: "EmbeddingGemma 300M",
		description:
			"Embed multilingual text for local semantic search with Google's 300M model.",
		previous_description:
			"Small, fast and multilingual. The best quality per megabyte we ship.",
		previous_introduction:
			"EmbeddingGemma is Google's 300M-parameter multilingual embedding model, built on Gemma 3 and trained across more than 100 languages.\n\n- Smallest capable option: 188 MB on disk, so it suits laptops and mobile devices.\n- Matches models five times its size on retrieval quality in our own testing.\n- Produces 768-dimensional vectors, the same width as GTE Multilingual, so it is a drop-in replacement for an existing index of that size.\n- Trades context for size: it reads up to 2,048 tokens per passage, where the other models here read far more.\n\nPick this one when index size and speed matter and your passages are short.",
		introduction:
			"Embed multilingual text for local semantic search with Google's 300M model. This Bit produces 768-dimensional vectors. The configured input limit is 2,048 tokens.\n\nUse the same model and compatible task prompts when embedding documents and queries. Rebuild the index when changing embedding models, even when their vector dimensions match.\n\nModel reference: https://huggingface.co/google/embeddinggemma-300m",
	},
	cunjxwl4c42jqz9c1t2ovoc3: {
		model_id: null,
		repository: "https://huggingface.co/Alibaba-NLP/gte-multilingual-base",
		file_name: "model_quantized.onnx",
		sources: ["https://huggingface.co/Alibaba-NLP/gte-multilingual-base"],
		name: "GTE Multilingual Base",
		description:
			"Embed multilingual queries and passages for local semantic search.",
		previous_description: "Strong multilingual model for local inference.",
		previous_introduction:
			"The gte-multilingual-base model is the latest in the GTE (General Text Embedding) family of models, featuring several key attributes:\n\n- High Performance: Achieves state-of-the-art (SOTA) results in multilingual retrieval tasks and multi-task representation model evaluations when compared to models of similar size.\n- Training Architecture: Trained using an encoder-only transformers architecture, resulting in a smaller model size. Unlike previous models based on decode-only LLM architecture (e.g., gte-qwen2-1.5b-instruct), this model has lower hardware requirements for inference, offering a 10x increase in inference speed.\n- Long Context: Supports text lengths up to 8192 tokens.\n- Multilingual Capability: Supports over 70 languages.\n- Elastic Dense Embedding: Support elastic output dense representation while maintaining the effectiveness of downstream tasks, which significantly reduces storage costs and improves execution efficiency.\n- Sparse Vectors: In addition to dense representations, it can also generate sparse vectors.",
		introduction:
			"Embed multilingual queries and passages for local semantic search. This Bit produces 768-dimensional vectors. The configured input limit is 8,192 tokens.\n\nUse the same model and compatible task prompts when embedding documents and queries. Rebuild the index when changing embedding models, even when their vector dimensions match.\n\nModel reference: https://huggingface.co/Alibaba-NLP/gte-multilingual-base",
	},
	d7bvkpc7pdlf64h3l0w0049m: {
		model_id: null,
		repository:
			"https://huggingface.co/ibm-granite/granite-embedding-311m-multilingual-r2",
		file_name: "model_quint8_avx2.onnx",
		sources: [
			"https://huggingface.co/ibm-granite/granite-embedding-311m-multilingual-r2",
		],
		name: "Granite Embedding 311M Multilingual R2",
		description:
			"Embed multilingual text and code with IBM's 311M retrieval model.",
		previous_description:
			"The strongest choice when questions and answers are in different languages.",
		previous_introduction:
			"IBM's Granite Embedding R2, covering more than 200 languages with focused support for 52 of them plus source code.\n\n- By a wide margin the best model here at matching a question in one language to its answer in another. In our testing it scored 0.89 where GTE Multilingual scored 0.67.\n- Reads up to 32,768 tokens per passage.\n- Emits 768-dimensional vectors, so it drops into an existing GTE index without rebuilding at a new width.\n- 298 MB on disk.\n\nNeeds no query or passage prefix. Pick this one for a knowledge base your team searches in several languages.",
		introduction:
			"Embed multilingual text and code with IBM's 311M retrieval model. This Bit produces 768-dimensional vectors. The configured input limit is 32,768 tokens.\n\nUse the same model and compatible task prompts when embedding documents and queries. Rebuild the index when changing embedding models, even when their vector dimensions match.\n\nModel reference: https://huggingface.co/ibm-granite/granite-embedding-311m-multilingual-r2",
	},
	gccb4h3bj8jdxxis2vcjuxuk: {
		model_id: null,
		repository:
			"https://huggingface.co/intfloat/multilingual-e5-large-instruct",
		file_name: "model.onnx",
		sources: ["https://huggingface.co/intfloat/multilingual-e5-large-instruct"],
		name: "Multilingual E5 Large Instruct",
		description:
			"Embed multilingual passages with task instructions for retrieval. Inputs are limited to 512 tokens.",
		previous_description:
			"The best multilingual model we have. Only available for paragraphs up to 512 tokens.",
		previous_introduction:
			"Multilingual - E5 - Large Instruct is the best retrieval model you can use for your case in multilingual context.",
		introduction:
			"Embed multilingual passages with task instructions for retrieval. Inputs are limited to 512 tokens. This Bit produces 1,024-dimensional vectors. The configured input limit is 512 tokens.\n\nUse the same model and compatible task prompts when embedding documents and queries. Rebuild the index when changing embedding models, even when their vector dimensions match.\n\nModel reference: https://huggingface.co/intfloat/multilingual-e5-large-instruct",
	},
	jw36q06zmu8rh2v6c7fvos12: {
		model_id: null,
		repository: "https://huggingface.co/jinaai/jina-embeddings-v2-base-de",
		file_name: "model_quantized.onnx",
		sources: ["https://huggingface.co/jinaai/jina-embeddings-v2-base-de"],
		name: "Jina Embeddings v2 Base DE",
		description:
			"Embed German and English text for semantic search and similarity matching.",
		previous_description: "Great German and English Model",
		previous_introduction: "Great German and English Model",
		introduction:
			"Embed German and English text for semantic search and similarity matching. This Bit produces 768-dimensional vectors. The configured input limit is 8,192 tokens.\n\nUse the same model and compatible task prompts when embedding documents and queries. Rebuild the index when changing embedding models, even when their vector dimensions match.\n\nModel reference: https://huggingface.co/jinaai/jina-embeddings-v2-base-de",
	},
	l6qsr57or0v8npz5i9fetizo: {
		model_id: null,
		repository: "https://huggingface.co/jinaai/jina-embeddings-v2-base-es",
		file_name: "model_quantized.onnx",
		sources: ["https://huggingface.co/jinaai/jina-embeddings-v2-base-es"],
		name: "Jina Embeddings v2 Base ES",
		description:
			"Embed Spanish and English text for semantic search and similarity matching.",
		previous_description: "Great spanish and english model.",
		previous_introduction: "Small and fast model for local embedding.",
		introduction:
			"Embed Spanish and English text for semantic search and similarity matching. This Bit produces 768-dimensional vectors. The configured input limit is 8,192 tokens.\n\nUse the same model and compatible task prompts when embedding documents and queries. Rebuild the index when changing embedding models, even when their vector dimensions match.\n\nModel reference: https://huggingface.co/jinaai/jina-embeddings-v2-base-es",
	},
	lo2dywqbrsgoxropzv0l0f7p: {
		model_id: null,
		repository: "https://huggingface.co/jinaai/jina-embeddings-v2-base-code",
		file_name: "model_quantized.onnx",
		sources: ["https://huggingface.co/jinaai/jina-embeddings-v2-base-code"],
		name: "Jina Embeddings v2 Base Code",
		description:
			"Find source code from English queries, or retrieve English descriptions from code.",
		previous_description:
			"Good in scenarios where Code Embedding is necessary.",
		previous_introduction: "Supports bidirectional English <-> Code searches.",
		introduction:
			"Find source code from English queries, or retrieve English descriptions from code. This Bit produces 768-dimensional vectors. The configured input limit is 8,192 tokens.\n\nUse the same model and compatible task prompts when embedding documents and queries. Rebuild the index when changing embedding models, even when their vector dimensions match.\n\nModel reference: https://huggingface.co/jinaai/jina-embeddings-v2-base-code",
	},
	rkhhottkdbbybjsplrxfmnhk: {
		model_id: null,
		repository:
			"https://huggingface.co/ibm-granite/granite-embedding-97m-multilingual-r2",
		file_name: "model_quint8_avx2.onnx",
		sources: [
			"https://huggingface.co/ibm-granite/granite-embedding-97m-multilingual-r2",
		],
		name: "Granite Embedding 97M Multilingual R2",
		description:
			"Embed multilingual text with IBM's compact 97M retrieval model.",
		previous_description:
			"The smallest and fastest model we ship, and still multilingual.",
		previous_introduction:
			"The 97M-parameter member of IBM's Granite Embedding R2 family.\n\n- 93 MB on disk and roughly three times faster per passage than anything else here.\n- Emits 384-dimensional vectors, halving the storage an index needs compared with a 768-dimensional model.\n- Keeps most of the cross-language strength of its larger sibling.\n- Reads up to 32,768 tokens per passage.\n- Ranks below the larger models on retrieval accuracy, which is the trade you are making.\n\nNeeds no query or passage prefix. Pick this one for mobile, for very large indexes, or wherever indexing throughput is the constraint.",
		introduction:
			"Embed multilingual text with IBM's compact 97M retrieval model. This Bit produces 384-dimensional vectors. The configured input limit is 32,768 tokens.\n\nUse the same model and compatible task prompts when embedding documents and queries. Rebuild the index when changing embedding models, even when their vector dimensions match.\n\nModel reference: https://huggingface.co/ibm-granite/granite-embedding-97m-multilingual-r2",
	},
	s63sr9z1gdgxr04vcznqihhk: {
		model_id: null,
		repository: "https://huggingface.co/microsoft/harrier-oss-v1-0.6b",
		file_name: "model-ffn_skip.onnx",
		sources: ["https://huggingface.co/microsoft/harrier-oss-v1-0.6b"],
		name: "Harrier OSS v1 0.6B",
		description:
			"Embed queries and passages with Microsoft's instruction-aware retrieval model.",
		previous_description:
			"Microsoft's retrieval model. Strongest on English, at the cost of size.",
		previous_introduction:
			"Harrier is Microsoft's open embedding model, released under the MIT licence.\n\n- Joint top of our retrieval benchmark alongside Qwen.\n- Reads up to 32,768 tokens per passage and emits 1,024-dimensional vectors.\n- Strongest of everything we tested at ranking English passages.\n- Weakest of the multilingual options at matching a question in one language to an answer in another, so it is a poor fit for a mixed-language index.\n- The largest bit here at 1.3 GB.\n\nThe query side is prefixed with a retrieval instruction automatically. Pick this one for English-first search where accuracy outranks download size.",
		introduction:
			"Embed queries and passages with Microsoft's instruction-aware retrieval model. This Bit produces 1,024-dimensional vectors. The configured input limit is 32,768 tokens.\n\nUse the same model and compatible task prompts when embedding documents and queries. Rebuild the index when changing embedding models, even when their vector dimensions match.\n\nModel reference: https://huggingface.co/microsoft/harrier-oss-v1-0.6b",
	},
	spjpy0dfsjrhjl1m1prshox1: {
		model_id: null,
		repository: "https://huggingface.co/jinaai/jina-embeddings-v2-base-zh",
		file_name: "model_quantized.onnx",
		sources: ["https://huggingface.co/jinaai/jina-embeddings-v2-base-zh"],
		name: "Jina Embeddings v2 Base ZH",
		description:
			"Embed Chinese and English text for semantic search and similarity matching.",
		previous_description: "Great english and chinese model.",
		previous_introduction: "Great for fast local embeddings.",
		introduction:
			"Embed Chinese and English text for semantic search and similarity matching. This Bit produces 768-dimensional vectors. The configured input limit is 8,192 tokens.\n\nUse the same model and compatible task prompts when embedding documents and queries. Rebuild the index when changing embedding models, even when their vector dimensions match.\n\nModel reference: https://huggingface.co/jinaai/jina-embeddings-v2-base-zh",
	},
	z7xxufugq7mxomsx1mi4f1km: {
		model_id: null,
		repository: "https://huggingface.co/Qwen/Qwen3-Embedding-0.6B",
		file_name: "model_q4.onnx",
		sources: ["https://huggingface.co/Qwen/Qwen3-Embedding-0.6B"],
		name: "Qwen3 Embedding 0.6B",
		description:
			"Embed multilingual text and source code with task-specific retrieval instructions.",
		previous_description:
			"Instruction-tuned retrieval across 100+ languages and code, with a very long context.",
		previous_introduction:
			"Qwen3-Embedding is the highest scoring open embedding family on the multilingual MTEB leaderboard. This is the 0.6B member, quantised to run locally.\n\n- Joint top of our retrieval benchmark alongside Harrier.\n- Reads up to 32,768 tokens per passage, sixteen times the Gemma model.\n- Covers natural language and source code in one vector space.\n- Emits 1,024-dimensional vectors, so an index built on it is wider and more expensive to store than a 768-dimensional one.\n\nThe query side is prefixed with a retrieval instruction automatically. Pick this one for the best all-round accuracy when you can afford the index width.",
		introduction:
			"Embed multilingual text and source code with task-specific retrieval instructions. This Bit produces 1,024-dimensional vectors. The configured input limit is 8,192 tokens.\n\nUse the same model and compatible task prompts when embedding documents and queries. Rebuild the index when changing embedding models, even when their vector dimensions match.\n\nModel reference: https://huggingface.co/Qwen/Qwen3-Embedding-0.6B",
	},
	i6un3u7arw9mt59xzpc6rmzm: {
		model_id: "onnx-community/embeddinggemma-2-ONNX",
		repository: "https://huggingface.co/onnx-community/embeddinggemma-2-ONNX",
		file_name: null,
		sources: ["https://huggingface.co/onnx-community/embeddinggemma-2-ONNX"],
		name: "EmbeddingGemma 2",
	},
	nu7qqwz0f906kxvpvfpftims: {
		model_id: "perplexity-ai/pplx-embed-v1-0.6b",
		repository: "https://huggingface.co/perplexity-ai/pplx-embed-v1-0.6b",
		file_name: null,
		sources: ["https://huggingface.co/perplexity-ai/pplx-embed-v1-0.6b"],
		name: "Perplexity Embed v1 0.6B",
	},
	k10qlw21o4ct379j3lpy7fo1: {
		model_id: null,
		repository: "https://huggingface.co/Qdrant/clip-ViT-B-32-vision",
		file_name: "model.onnx",
		sources: ["https://huggingface.co/Qdrant/clip-ViT-B-32-vision"],
		name: "CLIP ViT-B/32 Vision",
		description:
			"Embed images for visual search. Pair this encoder with the matching CLIP ViT-B/32 text model.",
		previous_description:
			"CLIP connects images and text in the same “language.” It lets you search for images by typing words, or describe an image and find related content instantly.",
		previous_introduction:
			"CLIP is an AI model created to understand pictures and text together. It learns how to match what it “sees” in an image with the words we use to describe it. This makes it powerful for tasks like searching for photos using natural language (e.g. “a dog playing frisbee”), automatically tagging images with meaningful labels, or finding pictures similar to a description. Instead of relying on fixed categories, CLIP understands concepts more flexibly — closer to how people think about visuals and words.",
		introduction:
			"Embed images for visual search. Pair this encoder with the matching CLIP ViT-B/32 text model. Use compatible image and text encoders when searching images with written queries.\n\nModel reference: https://huggingface.co/Qdrant/clip-ViT-B-32-vision",
	},
	s14ofcopraxeyu7dzx34hnwo: {
		model_id: null,
		repository: "https://huggingface.co/nomic-ai/nomic-embed-vision-v1.5",
		file_name: "model.onnx",
		sources: ["https://huggingface.co/nomic-ai/nomic-embed-vision-v1.5"],
		name: "Nomic Embed Vision v1.5",
		description:
			"Embed images for visual search in the shared space of Nomic Embed Text v1.5.",
		previous_description:
			"Nomic Embed creates powerful fingerprints of images and text, placing them in the same space so you can instantly search, compare, or organize content by meaning instead of exact words.",
		previous_introduction:
			"Nomic Embed is a modern AI model designed to understand both images and text in a unified way. It doesn’t just look at pictures or words literally—it captures their meaning and places them on the same map. This means you can type a phrase like “a cozy living room” and quickly find matching photos, or group together similar items without predefined tags. It’s especially useful for organizing large collections of media, making recommendations, or building smarter search features that feel natural and intuitive.",
		introduction:
			"Embed images for visual search in the shared space of Nomic Embed Text v1.5. Use compatible image and text encoders when searching images with written queries.\n\nModel reference: https://huggingface.co/nomic-ai/nomic-embed-vision-v1.5",
	},
	cis3ujaqj8yx0fpfgoye1prn: {
		model_id: null,
		repository:
			"https://huggingface.co/canavar/clip-ViT-B-32-multilingual-v1-ONNX/resolve/main/model.onnx?download=true",
		file_name: "model.onnx",
		sources: [
			"https://huggingface.co/sentence-transformers/clip-ViT-B-32-multilingual-v1",
		],
		name: "CLIP ViT-B/32 Multilingual Text",
		description:
			"Embed multilingual image-search queries in the CLIP ViT-B/32 visual space.",
		previous_description: "",
		previous_introduction: "",
		introduction:
			"Embed multilingual image-search queries in the CLIP ViT-B/32 visual space. This ONNX Bit produces 512-dimensional vectors with a configured input limit of 50 tokens.\n\nModel reference: https://huggingface.co/sentence-transformers/clip-ViT-B-32-multilingual-v1",
	},
	dztj51d8oexo6skxiy22tn28: {
		model_id: null,
		repository:
			"https://huggingface.co/nomic-ai/nomic-embed-text-v1.5/resolve/main/onnx/model.onnx",
		file_name: "model.onnx",
		sources: ["https://huggingface.co/nomic-ai/nomic-embed-text-v1.5"],
		name: "Nomic Embed Text v1.5",
		description:
			"Embed English queries and passages for semantic search with Nomic Embed Text v1.5.",
		previous_description: "",
		previous_introduction: "",
		introduction:
			"Embed English queries and passages for semantic search with Nomic Embed Text v1.5. This ONNX Bit produces 768-dimensional vectors with a configured input limit of 8,192 tokens.\n\nModel reference: https://huggingface.co/nomic-ai/nomic-embed-text-v1.5",
	},
	f6neh9w69tvfwqjuvde5yhsq: {
		model_id: null,
		repository:
			"https://huggingface.co/Qdrant/clip-ViT-B-32-text/resolve/main/model.onnx",
		file_name: "model.onnx",
		sources: ["https://huggingface.co/openai/clip-vit-base-patch32"],
		name: "CLIP ViT-B/32 Text",
		description:
			"Embed short English image descriptions for search with the matching CLIP ViT-B/32 image encoder.",
		previous_description: "",
		previous_introduction: "",
		introduction:
			"Embed short English image descriptions for search with the matching CLIP ViT-B/32 image encoder. This ONNX Bit produces 512-dimensional vectors with a configured input limit of 50 tokens.\n\nModel reference: https://huggingface.co/openai/clip-vit-base-patch32",
	},
	bkxmuuvmxhv1t1wffs6iz4nh: {
		model_id: "Qwen/Qwen3-ASR-1.7B",
		repository: "https://huggingface.co/Qwen/Qwen3-ASR-1.7B",
		file_name: "Qwen3-ASR-1.7B.stt.json",
		sources: ["https://huggingface.co/Qwen/Qwen3-ASR-1.7B"],
		description: "Transcribe spoken audio locally with Qwen3-ASR 1.7B.",
		previous_description:
			"Local Qwen3-ASR 1.7B multilingual speech-to-text model running on Candle.",
	},
	c13f0aunwurqrf3b7jiiv76o: {
		model_id: "openai/whisper-tiny",
		repository: "https://huggingface.co/openai/whisper-tiny",
		file_name: "whisper-tiny.stt.json",
		sources: ["https://huggingface.co/openai/whisper-tiny"],
		description: "Transcribe spoken audio locally with Whisper Tiny.",
		previous_description:
			"Local Whisper Tiny speech-to-text model running on Candle.",
	},
	ekcpr6ufyg8nr6wrwwc7tcm0: {
		model_id: "allenai/OLMoASR",
		repository: "https://huggingface.co/allenai/OLMoASR",
		file_name: "OLMoASR.stt.json",
		sources: ["https://huggingface.co/allenai/OLMoASR"],
		description: "Transcribe English audio locally with OLMoASR Base.",
		previous_description:
			"Local OLMoASR Base (English) English speech-to-text model running on Candle.",
	},
	euc892x55gg117ewhqh6pfn7: {
		model_id: "allenai/OLMoASR",
		repository: "https://huggingface.co/allenai/OLMoASR",
		file_name: "OLMoASR.stt.json",
		sources: ["https://huggingface.co/allenai/OLMoASR"],
		description: "Transcribe English audio locally with OLMoASR Small.",
		previous_description:
			"Local OLMoASR Small (English) English speech-to-text model running on Candle.",
	},
	ha7c0akiqbvxza3a04rru6z5: {
		model_id: "allenai/OLMoASR",
		repository: "https://huggingface.co/allenai/OLMoASR",
		file_name: "OLMoASR.stt.json",
		sources: ["https://huggingface.co/allenai/OLMoASR"],
		description: "Transcribe English audio locally with OLMoASR Medium.",
		previous_description:
			"Local OLMoASR Medium (English) English speech-to-text model running on Candle.",
	},
	hmaminmmjm6r1uoljw6crhqh: {
		model_id: "openai/whisper-large-v3-turbo",
		repository: "https://huggingface.co/openai/whisper-large-v3-turbo",
		file_name: "whisper-large-v3-turbo.stt.json",
		sources: ["https://huggingface.co/openai/whisper-large-v3-turbo"],
		description: "Transcribe spoken audio locally with Whisper Large v3 Turbo.",
		previous_description:
			"Local Whisper Large v3 Turbo speech-to-text model running on Candle.",
	},
	nxtfp7mg4uvzwyocygvzsif0: {
		model_id: "distil-whisper/distil-medium.en",
		repository: "https://huggingface.co/distil-whisper/distil-medium.en",
		file_name: "distil-medium.en.stt.json",
		sources: ["https://huggingface.co/distil-whisper/distil-medium.en"],
		description: "Transcribe English audio locally with Distil-Whisper Medium.",
		previous_description:
			"Local Distil-Whisper Medium (English) speech-to-text model running on Candle.",
	},
	o1390mhav3707n0evxqg642p: {
		model_id: "openai/whisper-medium",
		repository: "https://huggingface.co/openai/whisper-medium",
		file_name: "whisper-medium.stt.json",
		sources: ["https://huggingface.co/openai/whisper-medium"],
		description: "Transcribe spoken audio locally with Whisper Medium.",
		previous_description:
			"Local Whisper Medium speech-to-text model running on Candle.",
	},
	pc4n31oxjslcbt8rz7bw0s2c: {
		model_id: "openai/whisper-base",
		repository: "https://huggingface.co/openai/whisper-base",
		file_name: "whisper-base.stt.json",
		sources: ["https://huggingface.co/openai/whisper-base"],
		description: "Transcribe spoken audio locally with Whisper Base.",
		previous_description:
			"Local Whisper Base speech-to-text model running on Candle.",
	},
	qev9ifocbzr5b64cemnn8wxo: {
		model_id: "allenai/OLMoASR",
		repository: "https://huggingface.co/allenai/OLMoASR",
		file_name: "OLMoASR.stt.json",
		sources: ["https://huggingface.co/allenai/OLMoASR"],
		description: "Transcribe English audio locally with OLMoASR Large v2.",
		previous_description:
			"Local OLMoASR Large v2 (English) English speech-to-text model running on Candle.",
	},
	sfa53g81356j8jlntbsm6536: {
		model_id: "Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice",
		repository: "https://huggingface.co/Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice",
		file_name: "Qwen3-TTS-12Hz-1.7B-CustomVoice.tts.json",
		sources: ["https://huggingface.co/Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice"],
		description:
			"Generate multilingual speech locally with Qwen3-TTS CustomVoice and its preset voices.",
		previous_description:
			"Local Qwen3-TTS 12Hz 1.7B CustomVoice model for multilingual speech synthesis.",
	},
	sy42sai6ejy55mmf10vvevtz: {
		model_id: "hexgrad/Kokoro-82M",
		repository: "https://huggingface.co/hexgrad/Kokoro-82M",
		file_name: "Kokoro-82M.tts.json",
		sources: ["https://huggingface.co/hexgrad/Kokoro-82M"],
		description:
			"Generate English speech locally with Kokoro-82M and the bundled af_heart voice.",
		previous_description:
			"Local Kokoro-82M text-to-speech model for fast named-voice synthesis.",
	},
	t5gnja6eapi7fiu2ra3tdr07: {
		model_id: "UsefulSensors/moonshine-base",
		repository: "https://huggingface.co/UsefulSensors/moonshine-base",
		file_name: "moonshine-base.stt.json",
		sources: ["https://huggingface.co/UsefulSensors/moonshine-base"],
		description: "Transcribe English audio locally with Moonshine Base.",
		previous_description:
			"Local Moonshine Base English speech-to-text model running on Candle.",
	},
	ucn9kl0wdzl1ljbihdf10csg: {
		model_id: "distil-whisper/distil-large-v3",
		repository: "https://huggingface.co/distil-whisper/distil-large-v3",
		file_name: "distil-large-v3.stt.json",
		sources: ["https://huggingface.co/distil-whisper/distil-large-v3"],
		description:
			"Transcribe English audio locally with Distil-Whisper Large v3.",
		previous_description:
			"Local Distil-Whisper Large v3 speech-to-text model running on Candle.",
	},
	x7lyezo6xq83mdof2928m2t3: {
		model_id: "openai/whisper-small",
		repository: "https://huggingface.co/openai/whisper-small",
		file_name: "whisper-small.stt.json",
		sources: ["https://huggingface.co/openai/whisper-small"],
		description: "Transcribe spoken audio locally with Whisper Small.",
		previous_description:
			"Local Whisper Small speech-to-text model running on Candle.",
	},
	xh1abvs1xfjz52xs2lv3sdk2: {
		model_id: "allenai/OLMoASR",
		repository: "https://huggingface.co/allenai/OLMoASR",
		file_name: "OLMoASR.stt.json",
		sources: ["https://huggingface.co/allenai/OLMoASR"],
		description: "Transcribe English audio locally with OLMoASR Tiny.",
		previous_description:
			"Local OLMoASR Tiny (English) English speech-to-text model running on Candle.",
	},
	b8iq9nali7v6lkmubpzgr2sv: {
		model_id: "@preset/gemini-flash",
		repository: "https://flow-like.com/models",
		file_name: null,
		description:
			"Summarize documents and handle frequent workflow steps with the Gemini Flash preset. Catalog benchmark reference: Gemini 3.8 Flash.",
		previous_description:
			"Built for speed at scale. Reads very long documents in one go and answers in a fraction of the time a heavier model takes, which makes it the workhorse for high-volume flows.",
		sources: [
			"https://artificialanalysis.ai/models/gemini-3-8-flash",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/google/gemini-3.8-flash",
			"https://openrouter.ai/google/gemini-3.7-flash",
		],
		previous_introduction:
			"Built for speed at scale. Reads very long documents in one go and answers in a fraction of the time a heavier model takes, which makes it the workhorse for high-volume flows.",
		introduction:
			"Summarize documents and handle frequent workflow steps with the Gemini Flash preset. Verified routing is listed below. Runtime reasoning settings can differ from the benchmark configuration.",
		routing_note:
			"Verified preset routing (2026-11-16): primary model [Gemini 3.8 Flash](https://openrouter.ai/google/gemini-3.8-flash). If it fails, OpenRouter tries these fallbacks in order: [Gemini 3.7 Flash](https://openrouter.ai/google/gemini-3.7-flash). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
	},
	dt7121ak3swi6l45s3ei0g3e: {
		model_id: "@preset/gpt-5",
		repository: "https://flow-like.com/models",
		file_name: null,
		description:
			"Use the GPT preset for coding, document work, and reasoning. Catalog benchmark reference: GPT-6.1 Sol.",
		previous_description:
			"OpenAI model for coding, document work, and multi-step reasoning. This hosted option has a 272,000-token context limit to stay within the price ceiling.",
		sources: [
			"https://api.flow-like.com/api/v1/bit/dt7121ak3swi6l45s3ei0g3e?language=en",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/openai/gpt-6.1-sol",
			"https://openrouter.ai/openai/gpt-5.6-sol",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [GPT-6.1 Sol](https://openrouter.ai/openai/gpt-6.1-sol). If it fails, OpenRouter tries these fallbacks in order: [GPT-5.6 Sol](https://openrouter.ai/openai/gpt-5.6-sol). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
	},
	gji9xaeukf02wh7tzcq8rjqy: {
		model_id: "@preset/gpt-mini",
		repository: "https://flow-like.com/models",
		file_name: null,
		description:
			"Use GPT Mini for summaries, everyday coding, and frequent workflow steps. Catalog benchmark reference: GPT-6 Luna.",
		previous_description:
			"A smaller OpenAI model for frequent workflow steps, summaries, and everyday coding tasks.",
		sources: [
			"https://api.flow-like.com/api/v1/bit/gji9xaeukf02wh7tzcq8rjqy?language=en",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/openai/gpt-6-luna",
			"https://openrouter.ai/openai/gpt-5.6-luna",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [GPT-6 Luna](https://openrouter.ai/openai/gpt-6-luna). If it fails, OpenRouter tries these fallbacks in order: [GPT-5.6 Luna](https://openrouter.ai/openai/gpt-5.6-luna). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
	},
	q2wxeai8tg7uqwh426t2i2ta: {
		model_id: "@preset/gemini-flash-lite",
		repository: "https://flow-like.com/models",
		file_name: null,
		description:
			"Use Gemini Flash Lite for classification, text cleanup, translation, and short replies. Catalog benchmark reference: Gemini 3.5 Flash-Lite.",
		previous_description:
			"The quickest way to get a short answer. Trades depth for immediacy, so use it for classification, tidying text, translations and other simple steps that run often.",
		sources: [
			"https://artificialanalysis.ai/models/gemini-3-5-flash-lite",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/google/gemini-3.5-flash-lite",
			"https://openrouter.ai/google/gemini-3.1-flash-lite",
		],
		previous_introduction:
			"The quickest way to get a short answer. Trades depth for immediacy, so use it for classification, tidying text, translations and other simple steps that run often.",
		introduction:
			"Use Gemini Flash Lite for classification, text cleanup, translation, and short replies. Verified routing is listed below. Runtime reasoning settings can differ from the benchmark configuration.",
		routing_note:
			"Verified preset routing (2026-11-16): primary model [Gemini 3.5 Flash Lite](https://openrouter.ai/google/gemini-3.5-flash-lite). If it fails, OpenRouter tries these fallbacks in order: [Gemini 3.1 Flash Lite](https://openrouter.ai/google/gemini-3.1-flash-lite). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
	},
	rjnavenwqw016xjhctsnfpm1: {
		model_id: "@preset/kimi",
		repository: "https://flow-like.com/models",
		file_name: null,
		description:
			"Use the Kimi preset for coding and work with long documents. Catalog benchmark reference: Kimi K3.",
		previous_description:
			"Moonshot model for coding and multi-step work with long documents. Hosted requests use providers within the price ceiling.",
		sources: [
			"https://artificialanalysis.ai/models/kimi-k3",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/moonshotai/kimi-k3",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [Kimi K3](https://openrouter.ai/moonshotai/kimi-k3). The intelligence score uses the benchmark configuration or base-model proxy described above. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets",
	},
	s14lujkm2gut2mwg0zo3imxv: {
		model_id: "@preset/glm-5",
		repository: "https://flow-like.com/models",
		file_name: null,
		description:
			"Use the GLM preset for software engineering and multi-step text workflows. Catalog benchmark reference: GLM-5.3.",
		previous_description:
			"Z.ai model for software engineering and multi-step text workflows, with a context window of about one million tokens.",
		sources: [
			"https://api.flow-like.com/api/v1/bit/s14lujkm2gut2mwg0zo3imxv?language=en",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/z-ai/glm-5.3",
			"https://openrouter.ai/z-ai/glm-5.2",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [GLM 5.3](https://openrouter.ai/z-ai/glm-5.3). If it fails, OpenRouter tries these fallbacks in order: [GLM 5.2](https://openrouter.ai/z-ai/glm-5.2). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
	},
	sidy2hl6i3wlxun1dwrqm7jn: {
		model_id: "@preset/mi-mo-flash",
		repository: "https://flow-like.com/models",
		file_name: null,
		description:
			"Use MiMo Flash for frequent workflow steps with multimodal inputs. Catalog benchmark reference: MiMo-V2.6-Flash.",
		previous_description:
			"A lower-cost Xiaomi model for frequent workflow steps with text, images, audio, or video.",
		sources: [
			"https://artificialanalysis.ai/models/mimo-v2-6-flash",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/xiaomi/mimo-v2.6-flash",
			"https://openrouter.ai/xiaomi/mimo-v2.5",
			"https://openrouter.ai/xiaomi/mimo-v2.5-pro",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [MiMo-V2.6-Flash](https://openrouter.ai/xiaomi/mimo-v2.6-flash). If it fails, OpenRouter tries these fallbacks in order: [MiMo-V2.5](https://openrouter.ai/xiaomi/mimo-v2.5), [MiMo-V2.5-Pro](https://openrouter.ai/xiaomi/mimo-v2.5-pro). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
	},
	uf4euosbfjuqf889peep9fp0: {
		model_id: "@preset/mi-mo-pro",
		repository: "https://flow-like.com/models",
		file_name: null,
		description:
			"Use the MiMo preset for coding, reasoning, and multimodal workflows. Catalog benchmark reference: MiMo-V2.6-Pro.",
		previous_description:
			"Xiaomi model for coding, reasoning, and workflows with text, images, audio, or video.",
		sources: [
			"https://api.flow-like.com/api/v1/bit/uf4euosbfjuqf889peep9fp0?language=en",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/xiaomi/mimo-v2.6-pro",
			"https://openrouter.ai/xiaomi/mimo-v2.5-pro",
			"https://openrouter.ai/xiaomi/mimo-v2.5",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [MiMo-V2.6-Pro](https://openrouter.ai/xiaomi/mimo-v2.6-pro). If it fails, OpenRouter tries these fallbacks in order: [MiMo-V2.5-Pro](https://openrouter.ai/xiaomi/mimo-v2.5-pro), [MiMo-V2.5](https://openrouter.ai/xiaomi/mimo-v2.5). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
	},
	ul3hpasblgeapgja81sgwc0f: {
		model_id: "@preset/mini-max",
		repository: "https://flow-like.com/models",
		file_name: null,
		description:
			"Use the MiniMax preset for general workflow tasks and long inputs. Catalog benchmark reference: MiniMax-M3.",
		previous_description:
			"MiniMax model for general workflow tasks and long inputs, with a context window of about one million tokens.",
		sources: [
			"https://artificialanalysis.ai/models/minimax-m3",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/minimax/minimax-m3",
			"https://openrouter.ai/minimax/minimax-m2.7",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [MiniMax M3](https://openrouter.ai/minimax/minimax-m3). If it fails, OpenRouter tries these fallbacks in order: [MiniMax M2.7](https://openrouter.ai/minimax/minimax-m2.7). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
	},
	v2geho0wtkufwzh7e84pes2p: {
		model_id: "@preset/grok-fast",
		repository: "https://flow-like.com/models",
		file_name: null,
		description:
			"Use the Grok preset for coding, research, and long project documents. Catalog benchmark reference: Grok 4.7.",
		previous_description:
			"Grok for coding, research, and work with long project documents. Supports up to 500,000 tokens of context.",
		sources: [
			"https://api.flow-like.com/api/v1/bit/v2geho0wtkufwzh7e84pes2p?language=en",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/x-ai/grok-4.7",
			"https://openrouter.ai/x-ai/grok-4.6",
			"https://openrouter.ai/x-ai/grok-4.5",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [Grok 4.7](https://openrouter.ai/x-ai/grok-4.7). If it fails, OpenRouter tries these fallbacks in order: [Grok 4.6](https://openrouter.ai/x-ai/grok-4.6), [Grok 4.5](https://openrouter.ai/x-ai/grok-4.5). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
	},
	w11uweuf0nvqlvlc8avs5l7w: {
		model_id: "@preset/claude-haiku",
		repository: "https://flow-like.com/models",
		file_name: null,
		description:
			"Use Claude Haiku for drafting, summaries, and routine text tasks. Catalog benchmark reference: Claude Haiku 4.5.",
		previous_description:
			"The compact Claude. Anthropic's careful, well-mannered writing in a smaller and cheaper model, for quick drafting and tidy summaries.",
		sources: [
			"https://artificialanalysis.ai/models/claude-4-5-haiku",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/anthropic/claude-haiku-4.5",
		],
		previous_introduction:
			"The compact Claude. Anthropic's careful, well-mannered writing in a smaller and cheaper model, for quick drafting and tidy summaries.",
		introduction:
			"Use Claude Haiku for drafting, summaries, and routine text tasks. Verified routing is listed below. Runtime reasoning settings can differ from the benchmark configuration.",
		routing_note:
			"Verified preset routing (2026-11-16): primary model [Claude Haiku 4.5](https://openrouter.ai/anthropic/claude-haiku-4.5). The intelligence score uses the benchmark configuration or base-model proxy described above. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets",
	},
	zpn5p3hizsnlziu1po0g3wii: {
		model_id: "@preset/claude-sonnet",
		repository: "https://flow-like.com/models",
		file_name: null,
		description:
			"Use Claude Sonnet for coding, writing, and document work. Catalog benchmark reference: Claude Sonnet 5.5.",
		previous_description:
			"Claude for coding, writing, and document work, with a one-million-token context window.",
		sources: [
			"https://api.flow-like.com/api/v1/bit/zpn5p3hizsnlziu1po0g3wii?language=en",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/anthropic/claude-sonnet-5.5",
			"https://openrouter.ai/anthropic/claude-sonnet-5",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [Claude Sonnet 5.5](https://openrouter.ai/anthropic/claude-sonnet-5.5). If it fails, OpenRouter tries these fallbacks in order: [Claude Sonnet 5](https://openrouter.ai/anthropic/claude-sonnet-5). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
	},
	z7zt0kikpmxqlmbd885wfcjt: {
		model_id: "@preset/muse-spark",
		repository: "https://flow-like.com/models",
		file_name: null,
		description:
			"Use Muse Spark for reasoning, coding, and multimodal workflows. Catalog benchmark reference: Muse Spark 1.3.",
		previous_description:
			"Meta model for reasoning, coding, and work with text, images, files, and video.",
		sources: [
			"https://artificialanalysis.ai/models/muse-spark-1-3",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/meta/muse-spark-1.3",
			"https://openrouter.ai/meta/muse-spark-1.2",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [Muse Spark 1.3](https://openrouter.ai/meta/muse-spark-1.3). If it fails, OpenRouter tries these fallbacks in order: [Muse Spark 1.2](https://openrouter.ai/meta/muse-spark-1.2). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
	},
	hfxqyodgq5w607hssl9u0a57: {
		model_id: "qwen/qwen4-max",
		repository: "https://openrouter.ai/qwen/qwen4-max",
		file_name: null,
		sources: ["https://openrouter.ai/qwen/qwen4-max"],
		paragraph_replacements: [
			{
				previous:
					"The catalog uses OpenRouter's live endpoint limit of 131,072 output tokens and a total context window of 1,000,000 tokens. Standard rates are $1.50 per million input tokens and $4.50 per million output tokens. This is a proprietary hosted model.",
				replacement:
					"The catalog uses OpenRouter's endpoint limit of 131,072 output tokens and a total context window of 1,000,000 tokens. This is a proprietary hosted model. Provider rate references and Flow-Like usage charges are listed separately.",
			},
		],
	},
	qe5bda6hi3rhvqxwtl3xw227: {
		model_id: "openai/gpt-6-luna-decisions",
		repository: "https://openrouter.ai/openai/gpt-6-luna-decisions",
		file_name: null,
		sources: ["https://openrouter.ai/openai/gpt-6-luna-decisions"],
		paragraph_replacements: [
			{
				previous:
					"Provider pricing checked on 7 October 2026: $0.10 per million input tokens and no output-token charge. Flow-Like hosted usage charges also apply.",
				replacement:
					"Provider rates are shown as a reference. Flow-Like hosted usage charges also apply.",
			},
		],
	},
	a119qj7d82gufknkeuf2utcc: {
		model_id: "@preset/hy4",
		repository: "https://flow-like.com/models",
		file_name: null,
		sources: [
			"https://flow-like.com/models",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/tencent/hy4-preview",
			"https://openrouter.ai/tencent/hy3",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [Hy4 preview](https://openrouter.ai/tencent/hy4-preview). If it fails, OpenRouter tries these fallbacks in order: [Hy3](https://openrouter.ai/tencent/hy3). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
		description:
			"Use Hunyuan for Chinese and English text workflows, with HY4 Preview as the primary model and HY3 as fallback.",
	},
	bd7gty2o3i22holwbbcniknn: {
		model_id: "@preset/qwen-max-prime",
		repository: "https://flow-like.com/models",
		file_name: null,
		sources: [
			"https://flow-like.com/models",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/qwen/qwen3.8-max-prime",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [Qwen3.8 Max Prime](https://openrouter.ai/qwen/qwen3.8-max-prime). The intelligence score uses the benchmark configuration or base-model proxy described above. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets",
	},
	ca6ziza17c8xe6scwlnl42my: {
		model_id: "@preset/prod-free",
		repository: "https://flow-like.com/models",
		file_name: null,
		sources: [
			"https://flow-like.com/models",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/openrouter/free",
			"https://openrouter.ai/thinkingmachines/inkling:free",
			"https://openrouter.ai/deepseek/deepseek-v4.1-flash",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [Free Models Router](https://openrouter.ai/openrouter/free). If it fails, OpenRouter tries these fallbacks in order: [thinkingmachines/inkling:free](https://openrouter.ai/thinkingmachines/inkling:free), [DeepSeek V4.1 Flash](https://openrouter.ai/deepseek/deepseek-v4.1-flash). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
		description:
			"Try free models first, with paid DeepSeek V4.1 Flash as the final fallback. The serving model and usage charges can vary.",
		name: "Free Router with Paid Fallback",
		previous_introduction:
			"Free models, rate limited and shared. Best for trying things out. When the free pool is busy, requests fall through to a very cheap model so your flow keeps running.",
		introduction:
			"This preset tries OpenRouter Free first, then Inkling (free), and finally the paid DeepSeek V4.1 Flash model if earlier requests fail. Free routes are shared and rate limited. The paid fallback can incur usage charges; check the provider references and Flow-Like catalog tariff when planning usage.",
		paragraph_replacements: [
			{
				previous:
					"The route changes among free models and the catalog preset may use a cheap fallback. A conservative middle-of-pool estimate supports ranking, but there is no stable model or single benchmark score behind it; quality can vary substantially per request.",
				replacement:
					"This routing estimate is a conservative reference for the free-model pool. It does not measure every model that the preset can select, including the paid DeepSeek fallback. Quality can vary substantially per request.",
			},
		],
	},
	cmg7dolq3tdh2ew1d1jn7gjc: {
		model_id: "@preset/glm-prime",
		repository: "https://flow-like.com/models",
		file_name: null,
		sources: [
			"https://flow-like.com/models",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/z-ai/glm-5.3-prime",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [GLM 5.3 Prime](https://openrouter.ai/z-ai/glm-5.3-prime). The intelligence score uses the benchmark configuration or base-model proxy described above. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets",
	},
	ffffhlo5qj0ejwaedb4y2nmx: {
		model_id: "@preset/schematron",
		repository: "https://flow-like.com/models",
		file_name: null,
		sources: [
			"https://flow-like.com/models",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/inference-net/schematron-v2-turbo",
			"https://openrouter.ai/inference-net/schematron-v2-small",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [Schematron V2 Turbo](https://openrouter.ai/inference-net/schematron-v2-turbo). If it fails, OpenRouter tries these fallbacks in order: [Schematron V2 Small](https://openrouter.ai/inference-net/schematron-v2-small). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
		description:
			"Extract structured data with Schematron v2 Turbo, with Schematron v2 Small as a fallback.",
	},
	pzrr40v8j73qew036izh8329: {
		model_id: "@preset/kat-coder",
		repository: "https://flow-like.com/models",
		file_name: null,
		sources: [
			"https://flow-like.com/models",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/kwaipilot/kat-coder-pro-v2.5",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [kwaipilot/kat-coder-pro-v2.5](https://openrouter.ai/kwaipilot/kat-coder-pro-v2.5). The intelligence score uses the benchmark configuration or base-model proxy described above. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets",
	},
	q6x5uwsal7ben0aljtlmkbfk: {
		model_id: "@preset/qwen-flash",
		repository: "https://flow-like.com/models",
		file_name: null,
		sources: [
			"https://flow-like.com/models",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/qwen/qwen3.8-flash",
			"https://openrouter.ai/qwen/qwen3.7-flash",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [Qwen3.8 Flash](https://openrouter.ai/qwen/qwen3.8-flash). If it fails, OpenRouter tries these fallbacks in order: [Qwen3.7 Flash](https://openrouter.ai/qwen/qwen3.7-flash). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
		description:
			"Use Qwen Flash for text and image workflows. The route prioritizes Qwen3.8 Flash and can fall back to Qwen3.7 Flash.",
	},
	r10ut2atmhm52e3fu91kv4o6: {
		model_id: "@preset/mercury",
		repository: "https://flow-like.com/models",
		file_name: null,
		sources: [
			"https://flow-like.com/models",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/inception/mercury-2.5",
			"https://openrouter.ai/inception/mercury-2",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [inception/mercury-2.5](https://openrouter.ai/inception/mercury-2.5). If it fails, OpenRouter tries these fallbacks in order: [inception/mercury-2](https://openrouter.ai/inception/mercury-2). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
		description:
			"Use Mercury for text and coding tasks. The route prioritizes Mercury 2.5 and can fall back to Mercury 2.",
	},
	r13dd6mgmhqy17wsqx44k3mh: {
		model_id: "@preset/gemini-4-argon",
		repository: "https://flow-like.com/models",
		file_name: null,
		sources: [
			"https://flow-like.com/models",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/google/gemini-4-argon",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [Gemini 4 Argon](https://openrouter.ai/google/gemini-4-argon). The intelligence score uses the benchmark configuration or base-model proxy described above. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets",
		paragraph_replacements: [
			{
				previous:
					"Gemini 4 Argon is hosted through OpenRouter with a 1,048,576-token context window and text output. The model supports tool calling and structured output. The linked benchmark uses high reasoning effort; the hosted preset configuration has not been independently verified.",
				replacement:
					"Gemini 4 Argon is hosted through OpenRouter with a 1,048,576-token context window and text output. The model supports tool calling and structured output. The linked benchmark uses high reasoning effort. Runtime reasoning settings can differ from the benchmark configuration.",
			},
		],
	},
	rvdvssuvwvjt5dxpo3a2s48u: {
		model_id: "@preset/role-playing",
		repository: "https://flow-like.com/models",
		file_name: null,
		sources: [
			"https://flow-like.com/models",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/minimax/minimax-m2-her",
			"https://openrouter.ai/minimax/minimax-m3",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [minimax/minimax-m2-her](https://openrouter.ai/minimax/minimax-m2-her). If it fails, OpenRouter tries these fallbacks in order: [MiniMax M3](https://openrouter.ai/minimax/minimax-m3). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
		description:
			"Use MiniMax M2-her for character-based conversation, with MiniMax M3 as a fallback.",
	},
	t14ayybwpuqvjy9chcvcw16d: {
		model_id: "@preset/kimi-turbo",
		repository: "https://flow-like.com/models",
		file_name: null,
		sources: [
			"https://flow-like.com/models",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/moonshotai/kimi-k2.6",
			"https://openrouter.ai/moonshotai/kimi-k2.5",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [Kimi K2.6](https://openrouter.ai/moonshotai/kimi-k2.6). If it fails, OpenRouter tries these fallbacks in order: [Kimi K2.5](https://openrouter.ai/moonshotai/kimi-k2.5). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
		description:
			"Use Kimi K2.6 for coding and document tasks, with Kimi K2.5 as a fallback.",
	},
	tivq0reeun91llh5jt2iot9h: {
		model_id: "@preset/qwen-max",
		repository: "https://flow-like.com/models",
		file_name: null,
		sources: [
			"https://flow-like.com/models",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/qwen/qwen3.8-max-0902",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [Qwen3.8 Max (0902)](https://openrouter.ai/qwen/qwen3.8-max-0902). The intelligence score uses the benchmark configuration or base-model proxy described above. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets",
	},
	u10l52cpz538jc4ph081ijaq: {
		model_id: "@preset/glm-flash",
		repository: "https://flow-like.com/models",
		file_name: null,
		sources: [
			"https://flow-like.com/models",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/z-ai/glm-5.3-flash",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [GLM 5.3 Flash](https://openrouter.ai/z-ai/glm-5.3-flash). The intelligence score uses the benchmark configuration or base-model proxy described above. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets",
	},
	u7fqaks0zye6wlelc8fty8w7: {
		model_id: "@preset/glimpse",
		repository: "https://openrouter.ai/elorian/glimpse",
		file_name: null,
		sources: [
			"https://openrouter.ai/elorian/glimpse",
			"https://openrouter.ai/api/v1/presets",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [Glimpse](https://openrouter.ai/elorian/glimpse). The intelligence score uses the benchmark configuration or base-model proxy described above. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets",
	},
	upbus3wg4ar2nfosy2c5b5dp: {
		model_id: "@preset/laguna",
		repository: "https://flow-like.com/models",
		file_name: null,
		sources: [
			"https://flow-like.com/models",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/poolside/laguna-s-2.1",
			"https://openrouter.ai/poolside/laguna-xs-2.1",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [poolside/laguna-s-2.1](https://openrouter.ai/poolside/laguna-s-2.1). If it fails, OpenRouter tries these fallbacks in order: [poolside/laguna-xs-2.1](https://openrouter.ai/poolside/laguna-xs-2.1). The intelligence score describes its named benchmark reference. Fallback models can differ in quality, features and price. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets; fallback behavior: https://openrouter.ai/docs/guides/routing/model-fallbacks.md",
		description:
			"Use Poolside Laguna for coding and repository work. The route prioritizes Laguna S 2.1 and can fall back to Laguna XS 2.1.",
	},
	wyzvttfna07t9v9hiwdaxr6t: {
		model_id: "@preset/fugu-max",
		repository: "https://flow-like.com/models",
		file_name: null,
		sources: [
			"https://flow-like.com/models",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/sakana/fugu-max",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [Fugu Max](https://openrouter.ai/sakana/fugu-max). The intelligence score uses the benchmark configuration or base-model proxy described above. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets",
	},
	x74rocs5fucindudazh01trv: {
		model_id: "@preset/seed-code",
		repository: "https://flow-like.com/models",
		file_name: null,
		sources: [
			"https://flow-like.com/models",
			"https://openrouter.ai/api/v1/presets",
			"https://openrouter.ai/bytedance-seed/seed-2.0-code",
		],
		routing_note:
			"Verified preset routing (2026-11-16): primary model [bytedance-seed/seed-2.0-code](https://openrouter.ai/bytedance-seed/seed-2.0-code). The intelligence score uses the benchmark configuration or base-model proxy described above. Runtime reasoning settings can differ from the benchmark. Configuration source: https://openrouter.ai/api/v1/presets",
	},
};
