import { ADDITIONAL_EMBEDDING_RATINGS } from "./catalog-quality-embeddings";
import { SPECIALIZED_QUALITY_RATINGS } from "./catalog-quality-specialized";

export type QualityCategory =
	| "Tts"
	| "Stt"
	| "Embedding"
	| "ImageEmbedding"
	| "ImageGeneration"
	| "VideoGeneration"
	| "Forecasting"
	| "SystemOne";

export interface QualityEstimate {
	score: number;
	low: number;
	high: number;
	confidence: "low" | "medium";
	reasoning: string;
	sources: string[];
	updated_at: string;
	category: QualityCategory;
	criteria: string;
}

export interface CatalogQualityBit {
	type: string;
	repository?: string | null;
	parameters?: Record<string, unknown> | null;
}

export interface ReportedQualityBenchmark {
	name: string;
	score: number;
	unit: string;
	source: string;
	scope: string;
	notes: string;
	direction: "higher" | "lower";
}

export interface CatalogQualityRating {
	slug: string;
	name: string;
	creator_name: string;
	creator_slug: string;
	quality_estimate: QualityEstimate;
	reported_benchmarks?: ReportedQualityBenchmark[];
	model_ids: string[];
	repository_urls?: string[];
	model_type?: string;
}

const HF = "https://huggingface.co/";
const WHISPER = "https://github.com/openai/whisper";
const OLMO = `${HF}allenai/OLMoASR`;
const CRITERIA: Record<QualityCategory, string> = {
	ImageGeneration:
		"Visual quality, prompt adherence and editing consistency. Editorial 0–100 scale within image generation; compare similar output settings.",
	VideoGeneration:
		"Visual fidelity, temporal consistency and prompt adherence. Editorial 0–100 scale within video generation; motion, duration and resolution affect results.",
	Forecasting:
		"Forecast accuracy and generalization across time-series domains. Editorial 0–100 scale within forecasting; dataset, horizon and available history affect results.",
	SystemOne:
		"Accuracy and consistency of typed decisions about a state. Editorial 0–100 scale within decision models; evaluate on representative questions and states.",
	Tts: "Speech naturalness, intelligibility and expressive control in supported languages. Editorial 0–100 scale within text-to-speech; hardware speed and price are excluded.",
	Stt: "Transcription accuracy and robustness in supported languages. Editorial 0–100 scale within speech-to-text; hardware speed and price are excluded.",
	Embedding:
		"Retrieval relevance and semantic representation for the stated text domain. Editorial 0–100 scale within text embeddings; model-specific language and task limits still apply.",
	ImageEmbedding:
		"Image retrieval and image-text alignment. Editorial 0–100 scale within image embeddings; use the matching text encoder for text queries.",
};

interface RatingInput {
	key: string;
	name: string;
	creator: [string, string];
	category: QualityCategory;
	modelIds: string[];
	repositories?: string[];
	modelType?: string;
	score: number;
	range: [number, number];
	reasoning: string;
	sources: string[];
	confidence?: "low" | "medium";
	benchmarks?: ReportedQualityBenchmark[];
}

function rating(input: RatingInput): CatalogQualityRating {
	return {
		slug: `flow-like-estimate-quality-${input.key}`,
		name: input.name,
		creator_name: input.creator[0],
		creator_slug: input.creator[1],
		model_ids: input.modelIds,
		repository_urls:
			input.repositories ?? input.modelIds.map((id) => `${HF}${id}`),
		model_type: input.modelType,
		quality_estimate: {
			score: input.score,
			low: input.range[0],
			high: input.range[1],
			confidence: input.confidence ?? "low",
			reasoning: input.reasoning,
			sources: input.sources,
			updated_at: "2026-11-16",
			category: input.category,
			criteria: CRITERIA[input.category],
		},
		...(input.benchmarks ? { reported_benchmarks: input.benchmarks } : {}),
	};
}

const SPEECH_CREATORS: Record<string, string> = {
	assemblyai: "AssemblyAI",
	canopylabs: "Canopy Labs",
	deepgram: "Deepgram",
	elevenlabs: "ElevenLabs",
	"fish-audio": "Fish Audio",
	google: "Google",
	meta: "Meta",
	microsoft: "Microsoft AI",
	minimax: "MiniMax",
	mistralai: "Mistral AI",
	moondream: "Moondream",
	nvidia: "NVIDIA",
	openai: "OpenAI",
	qwen: "Qwen",
	sesame: "Sesame",
	"x-ai": "SpaceXAI",
};

// Reviewed against OpenRouter's dedicated speech/transcription model catalogs.
// Qualitative provider claims support placement, with broad editorial ranges.
const HOSTED_SPEECH_RATINGS = (
	[
		[
			"Tts",
			"qwen/qwen-audio-3.0-tts-plus",
			"Qwen-Audio-3.0-TTS Plus",
			90,
			"OpenRouter identifies Plus as the quality-oriented Qwen Audio 3.0 speech tier. The estimate favors narration quality over the Flash tier, but a common listening evaluation is unavailable; voice and language can change the result.",
		],
		[
			"Tts",
			"qwen/qwen-audio-3.0-tts-flash",
			"Qwen-Audio-3.0-TTS Flash",
			85,
			"The documented Flash tier targets responsive speech generation. Its position below Plus is an editorial inference from the provider's quality-tier distinction, with overlapping ranges because no shared listening test establishes the size of the difference.",
		],
		[
			"Tts",
			"deepgram/flux-tts",
			"Flux TTS",
			87,
			"The published model description emphasizes expressive English speech across the Flux voice catalog. This estimate applies to English delivery; voice choice and domain pronunciation need listening checks.",
		],
		[
			"Tts",
			"deepgram/aura-2",
			"Aura-2",
			85,
			"The published Aura-2 voice catalog supports multilingual synthesis. This estimate reflects general speech quality; the available documentation does not establish equal naturalness across all voices and languages.",
		],
		[
			"Tts",
			"mistralai/voxtral-mini-tts-3-0",
			"Voxtral TTS 3",
			91,
			"The model offers preset voices, voice-reference conditioning and delivery tags. These controls support expressive narration, but their availability and effectiveness vary with the request; the score is an editorial quality judgment.",
		],
		[
			"Tts",
			"mistralai/voxtral-mini-tts-2603",
			"Voxtral Mini TTS",
			86,
			"The documented model supports multilingual speech and voice-reference conditioning. This estimate covers general synthesis quality; voice similarity and naturalness have not been measured in this catalog.",
		],
		[
			"Tts",
			"canopylabs/orpheus-3b-0.1-ft",
			"Orpheus 3B",
			82,
			"The published model description highlights expressive English delivery with preset voices. This estimate covers English narration and dialogue; it provides no evidence for pronunciation quality in other languages.",
		],
		[
			"Tts",
			"sesame/csm-1b",
			"CSM 1B",
			81,
			"The model is designed for English conversational speech, with conversational and reading voices. The estimate favors dialogue use, while long narration and specialized vocabulary remain unmeasured here.",
		],
		[
			"Tts",
			"fish-audio/s1",
			"S1",
			87,
			"The model supports multilingual synthesis and controls for emotional delivery. This estimate reflects that expressive scope; actual naturalness depends on the language, chosen voice and instructions.",
		],
		[
			"Tts",
			"fish-audio/s2-pro",
			"S2 Pro",
			91,
			"The model supports expressive narration, dialogue with multiple speakers and style instructions. Its editorial placement reflects those capabilities; no common catalog listening test establishes a precise gap over S1.",
		],
		[
			"Tts",
			"fish-audio/s2.1-pro",
			"S2.1 Pro",
			93,
			"The published model description emphasizes multilingual narration and dialogue with flexible style instructions. This estimate is provisional because comparable listening results for the served model are limited.",
		],
		[
			"Tts",
			"fish-audio/s2.1-pro-free:free",
			"S2.1 Pro Free",
			93,
			"OpenRouter describes the free tier as having the same synthesis capabilities as S2.1 Pro. It receives the same output-quality estimate; differences in service availability and latency are outside this score.",
		],
		[
			"Tts",
			"fish-audio/drama-3",
			"Drama 3",
			94,
			"The model provides scene direction, pronunciation controls and multiple speakers, with cross-language delivery. These features support directed performances. Its published prompt examples also show uneven handling of numbers and units, so scripts still need listening checks.",
		],
		[
			"Tts",
			"elevenlabs/eleven-v4",
			"Eleven v4",
			95,
			"The model is documented as ElevenLabs' most expressive tier, with emotional delivery tags and broad language coverage. This editorial estimate favors controlled performances; it does not imply uniform pronunciation across languages or voices.",
		],
		[
			"Tts",
			"elevenlabs/eleven-v4-turbo",
			"Eleven v4 Turbo",
			94,
			"The provider describes Turbo as retaining v4's expressive controls while targeting faster speech generation. Its quality range overlaps v4 because the available evidence does not establish a large output-quality difference.",
		],
		[
			"Tts",
			"elevenlabs/eleven-v3",
			"Eleven v3",
			92,
			"The published model supports emotional delivery tags and multilingual speech. This estimate reflects expressive narration quality, with uncertainty for difficult pronunciation and long scripts.",
		],
		[
			"Tts",
			"elevenlabs/eleven-v3-conversational",
			"Eleven v3 Conversational",
			92,
			"The provider positions this v3 variant for natural dialogue. It receives the same quality estimate as v3, since a conversational specialization does not by itself establish superior speech quality.",
		],
		[
			"Tts",
			"elevenlabs/eleven-multilingual-v2",
			"Eleven Multilingual v2",
			88,
			"The model is documented for consistent narration and audiobooks across 29 languages. This estimate applies most directly to longer reading tasks; expressive dialogue may favor later models with delivery tags.",
		],
		[
			"Tts",
			"elevenlabs/eleven-turbo-v2.5",
			"Eleven Turbo v2.5",
			86,
			"The documented model balances delivery quality and responsiveness across 32 languages. The estimate covers general synthesis quality; language and voice selection may matter more than the small gap between Turbo and Flash.",
		],
		[
			"Tts",
			"elevenlabs/eleven-flash-v2.5",
			"Eleven Flash v2.5",
			85,
			"The model targets interactive speech across 32 languages. This provisional estimate reflects its conversational focus; faster service alone contributes no points to quality.",
		],
		[
			"Tts",
			"elevenlabs/eleven-turbo-v2",
			"Eleven Turbo v2",
			83,
			"The provider documents an English-only speech model intended for responsive applications. This estimate concerns English delivery; it should not be read as a multilingual capability rating.",
		],
		[
			"Tts",
			"elevenlabs/eleven-flash-v2",
			"Eleven Flash v2",
			82,
			"The published model targets English conversational speech. This estimate concerns naturalness and intelligibility, with a broad range because shared listening evidence for the served variant is limited.",
		],
		[
			"Tts",
			"assemblyai/assemblyai-tts-1",
			"AssemblyAI TTS 1",
			87,
			"The documented service supports narration and dialogue through a large voice library. Voice and language choices materially affect output; a large voice count does not establish uniform naturalness.",
		],
		[
			"Tts",
			"microsoft/mai-voice-2",
			"MAI-Voice-2",
			90,
			"The model is described for expressive dialogue and longer narration across 15 languages. This estimate reflects the intended speech quality; the catalog has not measured consistency across all supported voices.",
		],
		[
			"Tts",
			"microsoft/mai-voice-2-flash",
			"MAI-Voice-2-Flash",
			89,
			"The documented variant targets responsive, expressive speech across 15 languages. Its range overlaps MAI-Voice-2 because a latency distinction does not quantify a listening-quality difference.",
		],
		[
			"Tts",
			"microsoft/mai-voice-2.1",
			"MAI-Voice-2.1",
			94,
			"The provider identifies 2.1 as its fidelity-oriented speech tier, emphasizing prosody and speaker consistency across long narration in 23 languages. This remains a provisional editorial judgment without a common catalog listening test.",
		],
		[
			"Tts",
			"microsoft/mai-voice-2.1-flash",
			"MAI-Voice-2.1-Flash",
			92,
			"The published model combines expressive speech with an interactive focus across 23 languages. Its range overlaps the full 2.1 tier; responsiveness itself is excluded from the score.",
		],
		[
			"Tts",
			"minimax/speech-2.8-hd",
			"Speech 2.8 HD",
			90,
			"OpenRouter's shared prompt examples support narration quality, while number-heavy text and delivery cues show uneven results. This estimate is an editorial reading of that limited evidence, not a measured listening score.",
		],
		[
			"Tts",
			"minimax/speech-2.8-turbo",
			"Speech 2.8 Turbo",
			87,
			"OpenRouter's shared prompt examples support readable narration but show weaker mixed-language handling on some prompts. The broad range reflects the small sample and voice dependence.",
		],
		[
			"Tts",
			"x-ai/grok-voice-tts-1.0",
			"Grok Voice TTS 1.0",
			88,
			"The published model supports speech in more than twenty languages through five preset voices. This estimate covers general conversational delivery; language detection and a fixed voice catalog do not establish uniform naturalness.",
		],
		[
			"Stt",
			"nvidia/nemotron-3.5-asr-streaming-multilingual-0.6b",
			"Nemotron 3.5 ASR Streaming Multilingual 0.6B",
			87,
			"The model is designed for streaming multilingual recognition using a compact speech architecture. This provisional estimate covers transcription accuracy; streaming responsiveness and supported-language count are separate capabilities.",
		],
		[
			"Stt",
			"nvidia/nemotron-3.5-transcribe",
			"Nemotron 3.5 Transcribe",
			94,
			"The published model targets multilingual recognition over longer recordings with a larger mixture-of-experts architecture. Its high placement is provisional; capacity and context length alone do not establish accuracy on a user's recordings.",
		],
		[
			"Stt",
			"nvidia/parakeet-tdt-0.6b-v3",
			"Parakeet TDT 0.6B v3",
			88,
			"The published model provides multilingual recognition from a speech-specialized architecture. This estimate reflects general transcription quality in its supported languages; noise, accents and specialized terminology can change the ranking.",
		],
		[
			"Stt",
			"moondream/parakeet-ultra",
			"Parakeet Ultra",
			91,
			"OpenRouter reports lower transcription error rates than its Parakeet v3 base on English and multilingual test sets, including difficult recordings. The editorial score reflects that evidence; it does not guarantee the same improvement for every language.",
		],
		[
			"Stt",
			"deepgram/nova-3",
			"Nova-3",
			90,
			"The model is documented for general monolingual and multilingual transcription. This estimate reflects its broad transcription role; comparable accuracy evidence for the exact served configuration remains limited.",
		],
		[
			"Stt",
			"openai/whisper-1",
			"Whisper 1",
			82,
			"The published API model supports multilingual transcription and translation. This estimate reflects a mature general speech model; it should not inherit the scores of newer Whisper or GPT transcription variants.",
		],
		[
			"Stt",
			"openai/whisper-large-v3",
			"Whisper Large v3",
			90,
			"The model provides multilingual transcription and translation. The estimate places it above the compact Whisper variants for accuracy while retaining uncertainty for accents, noise and languages with less training coverage.",
		],
		[
			"Stt",
			"openai/gpt-transcribe",
			"GPT Transcribe",
			94,
			"The provider describes this model for accuracy-oriented transcription with context and keyword hints. This editorial estimate is provisional, since supported hints and input modes do not quantify transcription error rates.",
		],
		[
			"Stt",
			"mistralai/voxtral-mini-transcribe",
			"Voxtral Mini Transcribe",
			86,
			"The published model specializes the Voxtral Mini family for recordings such as meetings. This estimate covers general transcription quality; exact accuracy for the served checkpoint has not been independently checked here.",
		],
		[
			"Stt",
			"mistralai/voxtral-mini-3b-2507",
			"Voxtral Mini 3B 2507",
			82,
			"The model combines transcription, translation and audio understanding in a compact checkpoint. This estimate concerns transcription only; broad audio features do not establish the accuracy of individual words.",
		],
		[
			"Stt",
			"mistralai/voxtral-small-24b-2507-stt",
			"Voxtral Small 24B 2507 STT",
			87,
			"The documented larger Voxtral variant supports transcription and audio understanding. Its placement above Mini is a provisional capacity-based judgment, with overlapping uncertainty ranges rather than a claimed measured gap.",
		],
		[
			"Stt",
			"mistralai/voxtral-mini-transcribe-3-0",
			"Voxtral Mini Transcribe 3",
			90,
			"The model supports transcription with speaker labels, timestamps and domain-vocabulary biasing. This estimate reflects its specialized transcription role; metadata features alone do not establish word accuracy.",
		],
		[
			"Stt",
			"mistralai/voxtral-small-transcribe-3-0",
			"Voxtral Small Transcribe 3",
			94,
			"OpenRouter identifies Small Transcribe 3 as Mistral's flagship transcription tier, including support for domain vocabulary. This estimate favors accuracy-oriented use; the precise difference from Mini requires comparable audio tests.",
		],
		[
			"Stt",
			"assemblyai/universal-3-5-pro",
			"Universal-3.5 Pro",
			92,
			"The published model supports synchronous transcription with word timestamps for longer recordings. This estimate reflects its accuracy-oriented role; domain vocabulary and recording quality still require validation.",
		],
		[
			"Stt",
			"assemblyai/universal-3-6-pro",
			"Universal-3.6 Pro",
			95,
			"The provider reports improved short-conversation transcription over Universal-3.5 Pro. This estimate reflects that stated improvement while retaining uncertainty for other recording types and languages.",
		],
		[
			"Stt",
			"x-ai/grok-stt-1.0",
			"Grok STT 1.0",
			86,
			"The published model page documents transcription with timestamps and optional speaker labels. This estimate is provisional and covers the older 1.0 identity; it does not imply availability or inherit claims for Grok STT 2.0.",
		],
		[
			"Stt",
			"google/chirp-3",
			"Chirp 3",
			92,
			"The published model supports multilingual recognition with automatic punctuation and language detection. The estimate reflects general transcription quality; preview-language coverage should not be treated as equal accuracy across languages.",
		],
		[
			"Stt",
			"fish-audio/transcribe-1",
			"Transcribe 1",
			86,
			"The model supports automatic language detection and optional word alignment. This estimate covers general transcription; timestamp support does not establish word accuracy on noisy or overlapping speech.",
		],
		[
			"Stt",
			"fish-audio/transcribe-1-pro",
			"Transcribe 1 Pro",
			91,
			"The provider positions Pro for interviews, meetings and podcasts, including speaker and vocal-event cues. This estimate favors conversational recordings; those extra annotations do not independently prove transcript accuracy.",
		],
		[
			"Stt",
			"microsoft/mai-transcribe-1.5",
			"MAI-Transcribe 1.5",
			91,
			"The published model targets multilingual captions and call transcription across varied accents. This editorial score is provisional; quality can vary by language and recording domain.",
		],
		[
			"Stt",
			"microsoft/mai-transcribe-2",
			"MAI-Transcribe 2",
			95,
			"OpenRouter reports leading multilingual benchmark performance and support for mixed-language speech and vocabulary hints. The estimate reflects that evidence; a leaderboard claim does not establish equal accuracy across all sixty supported languages.",
		],
		[
			"Stt",
			"meta/muse-voice-transcribe-1.0",
			"Muse Voice Transcribe 1.0",
			89,
			"The published model targets push-to-talk and speaker-aware transcription with keyword and language hints. This estimate reflects interactive speech use; endpointing capabilities do not measure transcription accuracy.",
		],
		[
			"Stt",
			"qwen/qwen3-asr-0.6b",
			"Qwen3 ASR 0.6B",
			86,
			"The publisher's Qwen3-ASR comparison shows the compact model behind the 1.7B variant on several multilingual sets. This estimate reflects that tradeoff; performance still varies by language, dialect and decoding mode.",
		],
		[
			"Stt",
			"elevenlabs/scribe-v2",
			"Scribe v2",
			94,
			"The documented model supports multilingual transcription, timestamps, speaker labels and audio-event tags. This estimate reflects its specialized transcription role; annotation features are not a substitute for accuracy checks on a target dataset.",
		],
		[
			"Stt",
			"elevenlabs/scribe-v2-medical",
			"Scribe v2 Medical",
			94,
			"The provider documents tuning for clinical terminology. The score applies to that transcription domain and shares Scribe v2's broad range; it does not establish superiority for general recordings or certify clinical correctness.",
		],
	] as const
).map(([category, id, name, score, reasoning]) => {
	const owner = id.split("/")[0];
	return rating({
		key: `hosted-${id.replaceAll(/[^a-z0-9]+/g, "-")}`,
		name,
		creator: [SPEECH_CREATORS[owner] ?? owner, owner],
		category,
		modelIds: [id],
		repositories: [`https://openrouter.ai/${id}`],
		score,
		range: [score - 10, Math.min(99, score + 7)],
		reasoning: `Editorial estimate. ${reasoning}`,
		sources: [`https://openrouter.ai/${id}`],
	});
});

// These are editorial estimates informed by publisher evidence, not measured
// percentages. Ranges express judgment uncertainty, not statistical intervals.
// 90+ indicates leading quality, 80s strong, 70s capable, 60s baseline; lower
// scores reflect more substantial quality limits. Compare within a category.
export const CATALOG_QUALITY_RATINGS: CatalogQualityRating[] = [
	...HOSTED_SPEECH_RATINGS,
	...ADDITIONAL_EMBEDDING_RATINGS,
	...SPECIALIZED_QUALITY_RATINGS,
	rating({
		key: "kokoro-82m",
		name: "Kokoro-82M",
		creator: ["hexgrad", "hexgrad"],
		category: "Tts",
		modelIds: ["hexgrad/Kokoro-82M"],
		modelType: "Kokoro",
		score: 82,
		range: [74, 89],
		reasoning:
			"Editorial estimate for natural, intelligible narration from the published Kokoro model. Its compact design supports useful local speech quality, while voice, language and pronunciation choices affect results. The catalog bundles the af_heart voice; this score does not imply that every published voice is installed.",
		sources: [`${HF}hexgrad/Kokoro-82M`],
	}),
	rating({
		key: "qwen3-tts-1-7b-customvoice",
		name: "Qwen3-TTS 1.7B CustomVoice",
		creator: ["Qwen", "qwen"],
		category: "Tts",
		modelIds: ["Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice"],
		modelType: "Qwen3Tts",
		score: 91,
		range: [84, 96],
		confidence: "medium",
		reasoning:
			"Editorial estimate supported by the publisher's content-consistency and instruction-control evaluations. CustomVoice offers nine preset speakers and style instructions across ten languages. Use a speaker's native language for best results. This rating covers CustomVoice, whose capabilities differ from the Base voice-cloning model.",
		sources: [`${HF}Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice`],
		benchmarks: [
			{
				name: "English content consistency",
				score: 0.899,
				unit: "% WER",
				source: `${HF}Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice`,
				scope:
					"Publisher's English speech-generation evaluation, 12Hz 1.7B CustomVoice",
				notes:
					"Publisher-reported word error rate. Measures spoken-content consistency; it does not measure naturalness or establish quality in every language.",
				direction: "lower",
			},
		],
	}),
	...(
		[
			[
				"WhisperTiny",
				"tiny",
				50,
				40,
				62,
				"The smallest multilingual Whisper variant trades transcription accuracy for a small footprint. Suitable for clear speech when errors are tolerable; accents, noise and less represented languages can increase errors.",
			],
			[
				"WhisperBase",
				"base",
				59,
				48,
				70,
				"The base multilingual Whisper variant offers a modest accuracy step over tiny. Clear speech is its best fit; difficult audio and less represented languages still warrant review.",
			],
			[
				"WhisperSmall",
				"small",
				72,
				62,
				82,
				"The small multilingual Whisper variant improves the capacity available for transcription over tiny and base. Accuracy remains language-dependent, especially with noise or specialized vocabulary.",
			],
			[
				"WhisperMedium",
				"medium",
				82,
				73,
				89,
				"The medium multilingual Whisper variant is a stronger accuracy-oriented choice than the compact variants. Language coverage is broad, but the publisher reports substantial differences between languages.",
			],
			[
				"WhisperLargeV3Turbo",
				"large-v3-turbo",
				88,
				80,
				94,
				"The publisher describes turbo as a large-v3 derivative with a small accuracy loss. This estimate covers multilingual transcription. Turbo is not trained for translating non-English speech into English.",
			],
		] as const
	).map(([modelType, variant, score, low, high, reasoning]) =>
		rating({
			key: `whisper-${variant}`,
			name: `Whisper ${variant}`,
			creator: ["OpenAI", "openai"],
			category: "Stt",
			modelIds: [`openai/whisper-${variant}`],
			modelType,
			score,
			range: [low, high],
			reasoning: `Editorial estimate. ${reasoning}`,
			sources: [WHISPER],
		}),
	),
	rating({
		key: "distil-whisper-medium-en",
		name: "Distil-Whisper Medium English",
		creator: ["Hugging Face", "hugging-face"],
		category: "Stt",
		modelIds: ["distil-whisper/distil-medium.en"],
		modelType: "DistilWhisperMediumEn",
		score: 80,
		range: [72, 88],
		confidence: "medium",
		reasoning:
			"Editorial estimate for English transcription. The publisher reports accuracy close to its Whisper teacher on out-of-distribution speech after distillation. It targets English, so this score provides no evidence for multilingual transcription.",
		sources: [`${HF}distil-whisper/distil-medium.en`],
	}),
	rating({
		key: "distil-whisper-large-v3",
		name: "Distil-Whisper Large v3",
		creator: ["Hugging Face", "hugging-face"],
		category: "Stt",
		modelIds: ["distil-whisper/distil-large-v3"],
		modelType: "DistilWhisperLargeV3",
		score: 88,
		range: [81, 94],
		confidence: "medium",
		reasoning:
			"Editorial estimate for English transcription. The publisher reports results close to Whisper large-v3 on short and long recordings, with fewer hallucinations in its chunked long-form evaluation. Those results do not establish multilingual accuracy.",
		sources: [`${HF}distil-whisper/distil-large-v3`],
	}),
	rating({
		key: "moonshine-base-en",
		name: "Moonshine Base English",
		creator: ["Moonshine AI", "moonshine-ai"],
		category: "Stt",
		modelIds: ["UsefulSensors/moonshine-base", "moonshine-ai/moonshine-base"],
		modelType: "MoonshineBaseEn",
		score: 73,
		range: [63, 83],
		reasoning:
			"Editorial estimate for the original English-only Moonshine Base model. Its model card reports useful recognition accuracy in a compact model intended for local transcription. Noise, accents and hallucinated text remain limitations; this does not rate later Moonshine model generations.",
		sources: [`${HF}moonshine-ai/moonshine-base`],
	}),
	...(
		[
			["OlmoAsrTinyEn", "tiny", 51, 39, 64],
			["OlmoAsrBaseEn", "base", 60, 48, 73],
			["OlmoAsrSmallEn", "small", 73, 61, 84],
			["OlmoAsrMediumEn", "medium", 83, 72, 91],
			["OlmoAsrLargeEnV2", "large-v2", 89, 79, 95],
		] as const
	).map(([modelType, variant, score, low, high]) =>
		rating({
			key: `olmoasr-${variant}-en`,
			name: `OLMoASR ${variant} English`,
			creator: ["Ai2", "allenai"],
			category: "Stt",
			modelIds: ["allenai/OLMoASR"],
			modelType,
			score,
			range: [low, high],
			reasoning: `Editorial estimate for the English-only ${variant} checkpoint. The family uses open speech data and Whisper-style architectures. Relative placement follows the family's capacity tradeoff; the broad range reflects limited directly comparable checkpoint evidence.`,
			sources: [OLMO, "https://arxiv.org/abs/2508.20869"],
		}),
	),
	rating({
		key: "qwen3-asr-1-7b",
		name: "Qwen3-ASR 1.7B",
		creator: ["Qwen", "qwen"],
		category: "Stt",
		modelIds: ["Qwen/Qwen3-ASR-1.7B"],
		modelType: "Qwen3Asr17B",
		score: 93,
		range: [86, 97],
		confidence: "medium",
		reasoning:
			"Editorial estimate supported by the publisher's multilingual and difficult-audio evaluations, which show improvements over Whisper large-v3 on several tested sets. Scores vary by language and recording conditions; publisher benchmarks are not a validation of this local runtime.",
		sources: [`${HF}Qwen/Qwen3-ASR-1.7B`],
		benchmarks: [
			{
				name: "FLEURS multilingual transcription",
				score: 4.9,
				unit: "% WER",
				source: `${HF}Qwen/Qwen3-ASR-1.7B`,
				scope:
					"Publisher's 12-language FLEURS subset: en, zh, yue, ar, de, es, fr, it, ja, ko, pt, ru",
				notes:
					"Publisher-reported result for the 1.7B model. The expanded language subsets have different results; this is not a score across all supported languages.",
				direction: "lower",
			},
		],
	}),
	rating({
		key: "gte-multilingual-base",
		name: "GTE Multilingual Base",
		creator: ["Alibaba NLP", "alibaba-nlp"],
		category: "Embedding",
		modelIds: ["Alibaba-NLP/gte-multilingual-base"],
		score: 86,
		range: [77, 93],
		confidence: "medium",
		reasoning:
			"Editorial estimate based on the publisher's multilingual, cross-language and long-document retrieval evaluations. Broad language coverage and an 8,192-token input limit support general retrieval. Quality depends on using the intended pooling and normalization for the exported model.",
		sources: [`${HF}Alibaba-NLP/gte-multilingual-base`],
	}),
	rating({
		key: "multilingual-e5-large-instruct",
		name: "Multilingual E5 Large Instruct",
		creator: ["Microsoft Research", "microsoft"],
		category: "Embedding",
		modelIds: ["intfloat/multilingual-e5-large-instruct"],
		score: 88,
		range: [79, 94],
		confidence: "medium",
		reasoning:
			"Editorial estimate for multilingual semantic retrieval, supported by the model's published MTEB evaluations. Query instructions matter. Inputs are limited to 512 tokens, so longer documents need suitable chunks and this score does not imply full-document recall.",
		sources: [`${HF}intfloat/multilingual-e5-large-instruct`],
	}),
	...(
		[
			["de", "German", 79],
			["es", "Spanish", 79],
			["zh", "Chinese", 79],
		] as const
	).map(([code, language, score]) =>
		rating({
			key: `jina-v2-base-${code}`,
			name: `Jina v2 ${language} / English`,
			creator: ["Jina AI", "jina-ai"],
			category: "Embedding",
			modelIds: [`jinaai/jina-embeddings-v2-base-${code}`],
			score,
			range: [69, 88],
			reasoning: `Editorial estimate for ${language} and English retrieval. The bilingual training and published evaluations support this language pair, with input lengths up to 8,192 tokens. The equal ratings for the bilingual variants reflect insufficient common evidence to rank one language pair above another.`,
			sources: [`${HF}jinaai/jina-embeddings-v2-base-${code}`],
		}),
	),
	rating({
		key: "jina-v2-base-code",
		name: "Jina v2 Code",
		creator: ["Jina AI", "jina-ai"],
		category: "Embedding",
		modelIds: ["jinaai/jina-embeddings-v2-base-code"],
		score: 80,
		range: [69, 89],
		reasoning:
			"Editorial estimate for code search and technical English. Training uses code-question and docstring-code pairs across 30 programming languages. The model extrapolates its 512-token training length to longer inputs; relevance on large code files requires task-specific evaluation.",
		sources: [`${HF}jinaai/jina-embeddings-v2-base-code`],
	}),
	rating({
		key: "nomic-embed-text-v1-5",
		name: "Nomic Embed Text v1.5",
		creator: ["Nomic AI", "nomic-ai"],
		category: "Embedding",
		modelIds: ["nomic-ai/nomic-embed-text-v1.5"],
		score: 80,
		range: [71, 88],
		confidence: "medium",
		reasoning:
			"Editorial estimate for English semantic retrieval, informed by the publisher's MTEB results. Task prefixes and output dimension affect quality. Published results describe the reference model; this catalog's exported or remotely served configuration has not been independently evaluated.",
		sources: [`${HF}nomic-ai/nomic-embed-text-v1.5`],
		benchmarks: [
			{
				name: "MTEB",
				score: 62.28,
				unit: "points",
				source: `${HF}nomic-ai/nomic-embed-text-v1.5`,
				scope: "Publisher's reference v1.5 text model at 768 output dimensions",
				notes:
					"Original model-card MTEB aggregate. Lower output dimensions and other benchmark versions produce different scores. This is not an evaluation of the catalog runtime.",
				direction: "higher",
			},
		],
	}),
	rating({
		key: "clip-vit-b32-text",
		name: "CLIP ViT-B/32 Text",
		creator: ["OpenAI", "openai"],
		category: "Embedding",
		modelIds: ["Qdrant/clip-ViT-B-32-text"],
		score: 64,
		range: [51, 76],
		reasoning:
			"Editorial estimate for short English descriptions in image search. CLIP aligns text with visual concepts, which limits its fit for general document retrieval. Use its matching CLIP image encoder; the ONNX export has not been independently benchmarked here.",
		sources: [`${HF}openai/clip-vit-base-patch32`],
	}),
	rating({
		key: "clip-vit-b32-multilingual-text",
		name: "CLIP ViT-B/32 Multilingual Text",
		creator: ["Sentence Transformers", "sentence-transformers"],
		category: "Embedding",
		modelIds: [
			"canavar/clip-ViT-B-32-multilingual-v1-ONNX",
			"sentence-transformers/clip-ViT-B-32-multilingual-v1",
		],
		score: 67,
		range: [54, 79],
		reasoning:
			"Editorial estimate for multilingual image-search queries. Distillation aligns text in more than 50 languages with the original CLIP image encoder. This specialization limits its suitability for document search; the catalog's ONNX conversion has not been independently benchmarked here.",
		sources: [`${HF}sentence-transformers/clip-ViT-B-32-multilingual-v1`],
	}),
	rating({
		key: "clip-vit-b32-vision",
		name: "CLIP ViT-B/32 Vision",
		creator: ["OpenAI", "openai"],
		category: "ImageEmbedding",
		modelIds: ["Qdrant/clip-ViT-B-32-vision"],
		score: 73,
		range: [61, 83],
		reasoning:
			"Editorial estimate for general image search with a matching CLIP text encoder. The publisher identifies weaknesses in fine-grained recognition and object counting. Results for the reference CLIP model do not independently validate this ONNX export.",
		sources: [`${HF}openai/clip-vit-base-patch32`],
	}),
	rating({
		key: "nomic-embed-vision-v1-5",
		name: "Nomic Embed Vision v1.5",
		creator: ["Nomic AI", "nomic-ai"],
		category: "ImageEmbedding",
		modelIds: ["nomic-ai/nomic-embed-vision-v1.5"],
		score: 85,
		range: [76, 92],
		confidence: "medium",
		reasoning:
			"Editorial estimate informed by the publisher's ImageNet and DataComp results. The model aligns with Nomic Embed Text v1.5 for image-text retrieval. Classification results support general visual representation, but do not establish retrieval quality for a particular image collection.",
		sources: [`${HF}nomic-ai/nomic-embed-vision-v1.5`],
		benchmarks: [
			{
				name: "ImageNet zero-shot classification",
				score: 71.0,
				unit: "% accuracy",
				source: `${HF}nomic-ai/nomic-embed-vision-v1.5`,
				scope: "Publisher's reference Nomic Embed Vision v1.5 model",
				notes:
					"Publisher-reported image classification result. This measures a different task from image retrieval and does not validate the catalog runtime.",
				direction: "higher",
			},
		],
	}),
];

function object(value: unknown): Record<string, unknown> | null {
	return value !== null && typeof value === "object" && !Array.isArray(value)
		? (value as Record<string, unknown>)
		: null;
}

function category(type: string): QualityCategory | null {
	switch (type.replaceAll("_", "").toLowerCase()) {
		case "tts":
			return "Tts";
		case "stt":
			return "Stt";
		case "embedding":
			return "Embedding";
		case "imageembedding":
			return "ImageEmbedding";
		case "imagegeneration":
			return "ImageGeneration";
		case "videogeneration":
			return "VideoGeneration";
		case "forecasting":
			return "Forecasting";
		case "systemone":
			return "SystemOne";
		default:
			return null;
	}
}

function canonicalRepository(
	repository: string | null | undefined,
): string | null {
	if (!repository) return null;
	try {
		const url = new URL(repository);
		if (url.protocol !== "https:" || url.username || url.password || url.port)
			return null;
		if (!["huggingface.co", "openrouter.ai"].includes(url.hostname))
			return null;
		const [owner, model] = url.pathname.split("/").filter(Boolean);
		return owner && model
			? `${url.origin}/${owner}/${model}`.toLowerCase()
			: null;
	} catch {
		return null;
	}
}

/** Match exact model identities; unknown models remain unscored for review. */
export function findCatalogQualityRating(
	bit: CatalogQualityBit,
): CatalogQualityRating | null {
	const bitCategory = category(bit.type);
	if (!bitCategory) return null;
	const params = object(bit.parameters);
	const providerId = object(params?.provider)?.model_id;
	const modelId =
		typeof providerId === "string" && providerId.trim()
			? providerId.toLowerCase()
			: null;
	const repository = canonicalRepository(bit.repository);
	const modelType =
		typeof params?.model_type === "string" ? params.model_type : null;
	const matches = CATALOG_QUALITY_RATINGS.filter((entry) => {
		if (entry.quality_estimate.category !== bitCategory) return false;
		if (modelType && entry.model_type && modelType !== entry.model_type)
			return false;
		// OLMoASR shares one model repository across distinct checkpoints.
		if (
			entry.model_ids.includes("allenai/OLMoASR") &&
			modelType !== entry.model_type
		)
			return false;
		if (modelId)
			return entry.model_ids.some((id) => id.toLowerCase() === modelId);
		if (repository)
			return (
				entry.repository_urls?.some(
					(url) => canonicalRepository(url) === repository,
				) ?? false
			);
		return Boolean(
			modelType &&
				modelType === entry.model_type &&
				!providerId &&
				!bit.repository,
		);
	});
	return matches.length === 1 ? matches[0] : null;
}
