import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	type IBit,
	ITtsDTypePreference,
	type ITtsModelParameters,
	ITtsModelType,
	ITtsRuntimePreference,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	hfTtsAssetUrl,
	humanFileSize,
} from "@flow-like/flow-like-ui";
import type { Dispatch, SetStateAction } from "react";

export type TtsAssetDraft = {
	key: string;
	relativePath: string;
	required: boolean;
	bit: IBit;
};

type TtsPresetAsset = {
	relativePath: string;
	required?: boolean;
	size: number;
	sourcePath?: string;
	sourceRepo?: string;
};

type TtsModelPreset = {
	authors: string[];
	defaultLanguage: null | string;
	defaultVoice: null | string;
	description: string;
	docsUrl: string;
	label: string;
	languages: string[];
	license: string;
	longDescription: string;
	modelId: string;
	modelType: ITtsModelType;
	tags: string[];
	voices: string[];
	assets: TtsPresetAsset[];
};

const QWEN_0_5B_TOKENIZER_REPO = "Qwen/Qwen2.5-0.5B";
const QWEN_1_5B_TOKENIZER_REPO = "Qwen/Qwen2.5-1.5B";
const QWEN3_TTS_TOKENIZER_REPO = "Qwen/Qwen3-TTS-Tokenizer-12Hz";
const QWEN_TOKENIZER_JSON_SIZE = 7_031_645;

const TTS_MODEL_TYPES = Object.values(ITtsModelType);

const asset = (
	relativePath: string,
	size: number,
	options: Partial<Omit<TtsPresetAsset, "relativePath" | "size">> = {},
): TtsPresetAsset => ({
	relativePath,
	required: true,
	size,
	...options,
});

const voxtralVoiceAssets = [
	["ar_male", 413_253],
	["casual_female", 1_316_421],
	["casual_male", 904_773],
	["cheerful_female", 812_613],
	["de_female", 904_773],
	["de_male", 1_003_077],
	["es_female", 849_477],
	["es_male", 1_279_557],
	["fr_female", 597_573],
	["fr_male", 597_573],
	["hi_female", 529_989],
	["hi_male", 579_141],
	["it_female", 1_058_373],
	["it_male", 1_033_797],
	["neutral_female", 1_340_997],
	["neutral_male", 1_039_941],
	["nl_female", 898_629],
	["nl_male", 849_477],
	["pt_female", 1_076_805],
	["pt_male", 886_341],
] as const;

export const TTS_MODEL_PRESETS: Record<ITtsModelType, TtsModelPreset> = {
	[ITtsModelType.Kokoro]: {
		assets: [
			asset("config.json", 2_351),
			asset("kokoro-v1_0.pth", 327_212_226),
			asset("voices/af_heart.pt", 523_425),
		],
		authors: ["https://huggingface.co/hexgrad"],
		defaultLanguage: "en",
		defaultVoice: "af_heart",
		description:
			"Local Kokoro-82M text-to-speech model for fast named-voice synthesis.",
		docsUrl: "https://huggingface.co/hexgrad/Kokoro-82M",
		label: "Kokoro-82M",
		languages: [
			"en",
			"en-gb",
			"ja",
			"zh",
			"ko",
			"fr",
			"de",
			"it",
			"pt",
			"es",
			"hi",
		],
		license: "apache-2.0",
		longDescription:
			"Kokoro-82M is a compact StyleTTS2-style local TTS model. This preset installs the Kokoro config, PyTorch weights, and the af_heart voice pack expected by any-tts.",
		modelId: "hexgrad/Kokoro-82M",
		modelType: ITtsModelType.Kokoro,
		tags: ["text-to-speech", "local", "any-tts", "kokoro"],
		voices: ["af_heart"],
	},
	[ITtsModelType.OmniVoice]: {
		assets: [
			asset("config.json", 2_238),
			asset("tokenizer.json", 11_423_986),
			asset("model.safetensors", 2_450_344_112),
			asset("audio_tokenizer/config.json", 2_531),
			asset("audio_tokenizer/model.safetensors", 805_665_628),
		],
		authors: ["https://huggingface.co/k2-fsa"],
		defaultLanguage: "auto",
		defaultVoice: null,
		description:
			"Local OmniVoice multilingual zero-shot TTS model with instruction-based voice design.",
		docsUrl: "https://huggingface.co/k2-fsa/OmniVoice",
		label: "OmniVoice",
		languages: [
			"auto",
			"en",
			"zh",
			"ja",
			"ko",
			"de",
			"fr",
			"es",
			"pt",
			"ru",
			"it",
		],
		license: "apache-2.0",
		longDescription:
			"OmniVoice is a multilingual any-tts backend that uses text instructions instead of named voice files. This preset installs the main model, tokenizer, and audio tokenizer assets.",
		modelId: "k2-fsa/OmniVoice",
		modelType: ITtsModelType.OmniVoice,
		tags: ["text-to-speech", "local", "any-tts", "omnivoice", "multilingual"],
		voices: [],
	},
	[ITtsModelType.Qwen3Tts]: {
		assets: [
			asset("config.json", 4_908),
			asset("tokenizer.json", QWEN_TOKENIZER_JSON_SIZE, {
				sourceRepo: QWEN_0_5B_TOKENIZER_REPO,
			}),
			asset("model.safetensors", 3_833_402_552),
			asset("speech_tokenizer/model.safetensors", 682_293_092, {
				sourcePath: "model.safetensors",
				sourceRepo: QWEN3_TTS_TOKENIZER_REPO,
			}),
			asset("speech_tokenizer/config.json", 2_336, {
				required: false,
				sourcePath: "config.json",
				sourceRepo: QWEN3_TTS_TOKENIZER_REPO,
			}),
		],
		authors: ["https://huggingface.co/Qwen"],
		defaultLanguage: "auto",
		defaultVoice: "serena",
		description:
			"Local Qwen3-TTS 12Hz 1.7B CustomVoice model for multilingual speech synthesis.",
		docsUrl: "https://huggingface.co/Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice",
		label: "Qwen3-TTS 1.7B CustomVoice",
		languages: [
			"auto",
			"chinese",
			"english",
			"german",
			"italian",
			"portuguese",
			"spanish",
			"japanese",
			"korean",
			"french",
			"russian",
		],
		license: "apache-2.0",
		longDescription:
			"Qwen3-TTS CustomVoice is a 1.7B local TTS model. any-tts loads the Qwen2.5 tokenizer plus the Qwen3 talker and speech tokenizer assets.",
		modelId: "Qwen/Qwen3-TTS-12Hz-1.7B-CustomVoice",
		modelType: ITtsModelType.Qwen3Tts,
		tags: ["text-to-speech", "local", "any-tts", "qwen3-tts", "multilingual"],
		voices: [
			"serena",
			"vivian",
			"uncle_fu",
			"ryan",
			"aiden",
			"ono_anna",
			"sohee",
			"eric",
			"dylan",
		],
	},
	[ITtsModelType.VibeVoice]: {
		assets: [
			asset("config.json", 2_762),
			asset("tokenizer.json", QWEN_TOKENIZER_JSON_SIZE, {
				sourceRepo: QWEN_1_5B_TOKENIZER_REPO,
			}),
			asset("model-00001-of-00003.safetensors", 1_975_317_828),
			asset("model-00002-of-00003.safetensors", 1_983_051_688),
			asset("model-00003-of-00003.safetensors", 1_449_832_938),
			asset("preprocessor_config.json", 351, { required: false }),
		],
		authors: ["https://huggingface.co/microsoft"],
		defaultLanguage: "auto",
		defaultVoice: null,
		description:
			"Local VibeVoice 1.5B model for long-form and multi-speaker speech generation.",
		docsUrl: "https://huggingface.co/microsoft/VibeVoice-1.5B",
		label: "VibeVoice 1.5B",
		languages: ["auto", "multilingual"],
		license: "mit",
		longDescription:
			"VibeVoice-1.5B is a local long-form speech generation model. This preset installs the Microsoft VibeVoice config, sharded weights, preprocessor config, and the Qwen2.5 tokenizer required by any-tts.",
		modelId: "microsoft/VibeVoice-1.5B",
		modelType: ITtsModelType.VibeVoice,
		tags: ["text-to-speech", "local", "any-tts", "vibevoice", "long-form"],
		voices: [],
	},
	[ITtsModelType.VibeVoiceRealtime]: {
		assets: [
			asset("config.json", 2_117),
			asset("tokenizer.json", QWEN_TOKENIZER_JSON_SIZE, {
				sourceRepo: QWEN_0_5B_TOKENIZER_REPO,
			}),
			asset("model.safetensors", 2_035_332_888),
			asset("preprocessor_config.json", 360, { required: false }),
		],
		authors: ["https://huggingface.co/microsoft"],
		defaultLanguage: "auto",
		defaultVoice: null,
		description:
			"Local VibeVoice Realtime 0.5B model for streaming-oriented speech generation.",
		docsUrl: "https://huggingface.co/microsoft/VibeVoice-Realtime-0.5B",
		label: "VibeVoice Realtime 0.5B",
		languages: ["auto", "multilingual"],
		license: "mit",
		longDescription:
			"VibeVoice Realtime 0.5B is a streaming-oriented local TTS model. This preset installs the public model files and Qwen2.5 tokenizer expected by any-tts.",
		modelId: "microsoft/VibeVoice-Realtime-0.5B",
		modelType: ITtsModelType.VibeVoiceRealtime,
		tags: ["text-to-speech", "local", "any-tts", "vibevoice", "realtime"],
		voices: [],
	},
	[ITtsModelType.Voxtral]: {
		assets: [
			asset("params.json", 3_482),
			asset("tekken.json", 14_894_731),
			asset("consolidated.safetensors", 8_004_752_248),
			...voxtralVoiceAssets.map(([voice, size]) =>
				asset(`voice_embedding/${voice}.pt`, size),
			),
		],
		authors: ["https://huggingface.co/mistralai"],
		defaultLanguage: "en",
		defaultVoice: "neutral_male",
		description:
			"Local Voxtral 4B TTS model with multilingual preset voice embeddings.",
		docsUrl: "https://huggingface.co/mistralai/Voxtral-4B-TTS-2603",
		label: "Voxtral 4B TTS 2603",
		languages: ["en", "fr", "es", "de", "it", "pt", "nl", "ar", "hi"],
		license: "cc-by-nc-4.0",
		longDescription:
			"Voxtral-4B-TTS-2603 is a local Mistral TTS model. This preset installs params.json, tekken.json, the consolidated weights, and every preset voice embedding listed in the model config.",
		modelId: "mistralai/Voxtral-4B-TTS-2603",
		modelType: ITtsModelType.Voxtral,
		tags: ["text-to-speech", "local", "any-tts", "voxtral", "multilingual"],
		voices: voxtralVoiceAssets.map(([voice]) => voice),
	},
};

export function getTtsModelPreset(modelType: ITtsModelType): TtsModelPreset {
	return (
		TTS_MODEL_PRESETS[modelType] ?? TTS_MODEL_PRESETS[ITtsModelType.Kokoro]
	);
}

function ttsManifestFileName(modelId: string): string {
	const modelName = modelId.split("/").pop() || "tts-model";
	return `${modelName.replace(/[^a-zA-Z0-9._-]/g, "-")}.tts.json`;
}

function ttsAssetRefs(assets: TtsAssetDraft[]): ITtsModelParameters["assets"] {
	return assets.map((asset) => ({
		bit: asset.bit.id,
		relative_path: asset.relativePath,
		required: asset.required,
	}));
}

export function defaultTtsAssetLayout(
	modelType: ITtsModelType,
	createBit: () => IBit,
): TtsAssetDraft[] {
	const preset = getTtsModelPreset(modelType);

	return preset.assets.map((presetAsset) => {
		const sourceRepo = presetAsset.sourceRepo ?? preset.modelId;
		const sourcePath = presetAsset.sourcePath ?? presetAsset.relativePath;
		const downloadLink = hfTtsAssetUrl(sourceRepo, sourcePath);
		const fileName = sourcePath.split("/").pop() ?? "";
		const baseBit = createBit();

		return {
			key: crypto.randomUUID(),
			relativePath: presetAsset.relativePath,
			required: presetAsset.required ?? true,
			bit: {
				...baseBit,
				authors: preset.authors,
				download_link: downloadLink,
				file_name: fileName,
				license: preset.license,
				meta: {
					...baseBit.meta,
					en: {
						...baseBit.meta?.en,
						description: `${preset.label} asset: ${presetAsset.relativePath}`,
						name: `${preset.label} ${presetAsset.relativePath}`,
						tags: ["tts-asset", "any-tts", ...preset.tags],
					},
				},
				repository: `https://huggingface.co/${sourceRepo}`,
				size: presetAsset.size,
			},
		};
	});
}

export function applyTtsModelPreset(
	bit: IBit,
	modelType: ITtsModelType,
	assets: TtsAssetDraft[],
): IBit {
	const preset = getTtsModelPreset(modelType);
	const parameters = (bit.parameters ?? {}) as Partial<ITtsModelParameters>;

	return {
		...bit,
		authors: preset.authors,
		download_link: "",
		file_name: ttsManifestFileName(preset.modelId),
		license: preset.license,
		meta: {
			...bit.meta,
			en: {
				...bit.meta?.en,
				description: preset.description,
				docs_url: preset.docsUrl,
				long_description: preset.longDescription,
				name: preset.label,
				tags: preset.tags,
				website: preset.docsUrl,
			},
		},
		name: preset.label,
		parameters: {
			...parameters,
			assets: ttsAssetRefs(assets),
			default_language: preset.defaultLanguage,
			default_voice: preset.defaultVoice,
			dtype: ITtsDTypePreference.Auto,
			languages: preset.languages,
			model_type: modelType,
			provider: {
				...parameters.provider,
				model_id: preset.modelId,
				provider_name: "local:any-tts",
				version: null,
			},
			runtime: ITtsRuntimePreference.Auto,
			voices: preset.voices,
		},
		repository: preset.docsUrl,
		size: 0,
	};
}

export function TTSConfiguration({
	bit,
	setBit,
	assetBits,
	setAssetBits,
	createAssetBit,
}: Readonly<{
	bit: IBit;
	setBit: Dispatch<SetStateAction<IBit>>;
	assetBits: TtsAssetDraft[];
	setAssetBits: Dispatch<SetStateAction<TtsAssetDraft[]>>;
	createAssetBit: () => IBit;
}>) {
	const parameters = bit.parameters as ITtsModelParameters;
	const selectedModelType = parameters.model_type ?? ITtsModelType.Kokoro;
	const selectedPreset = getTtsModelPreset(selectedModelType);
	const totalAssetSize = assetBits.reduce(
		(total, asset) => total + (asset.bit.size ?? 0),
		0,
	);

	const applyPreset = (modelType: ITtsModelType) => {
		const assets = defaultTtsAssetLayout(modelType, createAssetBit);
		setAssetBits(assets);
		setBit((old) => applyTtsModelPreset(old, modelType, assets));
	};

	return (
		<div className="space-y-6 w-full max-w-screen-lg">
			<Card className="w-full">
				<CardHeader>
					<CardTitle>TTS Model</CardTitle>
					<CardDescription>
						{selectedPreset.description} {assetBits.length} files,{" "}
						{humanFileSize(totalAssetSize)}.
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-2">
					<Label htmlFor="tts-model-type">Model</Label>
					<Select
						value={selectedModelType}
						onValueChange={(value) => applyPreset(value as ITtsModelType)}
					>
						<SelectTrigger id="tts-model-type">
							<SelectValue placeholder="Select model" />
						</SelectTrigger>
						<SelectContent>
							{TTS_MODEL_TYPES.map((modelType) => {
								const preset = getTtsModelPreset(modelType);
								return (
									<SelectItem key={modelType} value={modelType}>
										{preset.label}
									</SelectItem>
								);
							})}
						</SelectContent>
					</Select>
				</CardContent>
			</Card>
		</div>
	);
}
