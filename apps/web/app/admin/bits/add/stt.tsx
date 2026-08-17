import { i18n as i18next, useTranslation } from "@flow-like/locales";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	type IBit,
	ISttDTypePreference,
	type ISttModelParameters,
	ISttModelType,
	ISttRuntimePreference,
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

// NOTE: asset byte sizes below are approximate and only used for display totals.
// The actual download uses the Content-Length reported by Hugging Face, so a
// rough value here never affects correctness.

export type SttAssetDraft = {
	key: string;
	relativePath: string;
	required: boolean;
	bit: IBit;
};

type SttPresetAsset = {
	relativePath: string;
	required?: boolean;
	size: number;
	sourcePath?: string;
	sourceRepo?: string;
};

type SttModelPreset = {
	authors: string[];
	defaultLanguage: null | string;
	description: string;
	docsUrl: string;
	label: string;
	languages: string[];
	license: string;
	longDescription: string;
	modelId: string;
	modelType: ISttModelType;
	tags: string[];
	assets: SttPresetAsset[];
};

const STT_MODEL_TYPES = Object.values(ISttModelType);

const MULTILINGUAL_LANGS = [
	"auto",
	"en",
	"zh",
	"de",
	"es",
	"ru",
	"ko",
	"fr",
	"ja",
	"pt",
	"tr",
	"pl",
	"nl",
	"ar",
	"it",
	"id",
	"hi",
];

const asset = (
	relativePath: string,
	size: number,
	options: Partial<Omit<SttPresetAsset, "relativePath" | "size">> = {},
): SttPresetAsset => ({
	relativePath,
	required: true,
	size,
	...options,
});

const whisperAssets = (
	modelSize: number,
	multilingual: boolean,
): SttPresetAsset[] => [
	asset("config.json", 2_000),
	asset("tokenizer.json", multilingual ? 2_200_000 : 836_000),
	asset("model.safetensors", modelSize),
	asset("generation_config.json", 4_000, { required: false }),
	asset("preprocessor_config.json", 340, { required: false }),
];

// OLMoASR ships a single Whisper-style checkpoint per size in the shared repo.
// TODO(any-speech-to-text): confirm the exact `*.pt` filenames in allenai/OLMoASR.
const OLMOASR_REPO = "allenai/OLMoASR";
const olmoAsrAssets = (ptFile: string, size: number): SttPresetAsset[] => [
	asset(ptFile, size, { sourceRepo: OLMOASR_REPO, sourcePath: ptFile }),
];

const whisperPreset = (
	modelType: ISttModelType,
	modelId: string,
	label: string,
	modelSize: number,
	multilingual: boolean,
): SttModelPreset => ({
	assets: whisperAssets(modelSize, multilingual),
	authors: ["https://huggingface.co/openai"],
	defaultLanguage: multilingual ? "auto" : "en",
	description: i18next.t('localLabelSpeechtotextModelRunningOnCandle', 'Local {{label}} speech-to-text model running on Candle.', { label }),
	docsUrl: `https://huggingface.co/${modelId}`,
	label,
	languages: multilingual ? MULTILINGUAL_LANGS : ["en"],
	license: "apache-2.0",
	longDescription: i18next.t('labelIsAWhisperTranscriptionModelLoadedLocallyThroughAnyspeechtotextThisPresetInstallsTheConfigTokenizerAndSafetensorsWeightsFromModelid', '{{label}} is a Whisper transcription model loaded locally through any-speech-to-text. This preset installs the config, tokenizer, and safetensors weights from {{modelId}}.', { label, modelId }),
	modelId,
	modelType,
	tags: [
		"speech-to-text",
		"local",
		"any-speech-to-text",
		"whisper",
		multilingual ? "multilingual" : "english",
	],
});

const olmoPreset = (
	modelType: ISttModelType,
	label: string,
	ptFile: string,
	size: number,
): SttModelPreset => ({
	assets: olmoAsrAssets(ptFile, size),
	authors: ["https://huggingface.co/allenai"],
	defaultLanguage: "en",
	description: i18next.t('localLabelEnglishSpeechtotextModelRunningOnCandle', 'Local {{label}} English speech-to-text model running on Candle.', { label }),
	docsUrl: "https://huggingface.co/allenai/OLMoASR",
	label,
	languages: ["en"],
	license: "apache-2.0",
	longDescription: i18next.t('labelIsAnOlmoasrWhisperarchitectureModelLoadedLocallyThroughAnyspeechtotextThisPresetInstallsThePtfileCheckpointFromAllenaiolmoasr', '{{label}} is an OLMoASR Whisper-architecture model loaded locally through any-speech-to-text. This preset installs the {{ptFile}} checkpoint from allenai/OLMoASR.', { label, ptFile }),
	modelId: OLMOASR_REPO,
	modelType,
	tags: ["speech-to-text", "local", "any-speech-to-text", "olmoasr", "english"],
});

export const STT_MODEL_PRESETS: Record<ISttModelType, SttModelPreset> = {
	[ISttModelType.WhisperTiny]: whisperPreset(
		ISttModelType.WhisperTiny,
		"openai/whisper-tiny",
		"Whisper Tiny",
		151_061_672,
		true,
	),
	[ISttModelType.WhisperTinyEn]: whisperPreset(
		ISttModelType.WhisperTinyEn,
		"openai/whisper-tiny.en",
		"Whisper Tiny (English)",
		151_061_672,
		false,
	),
	[ISttModelType.WhisperBase]: whisperPreset(
		ISttModelType.WhisperBase,
		"openai/whisper-base",
		"Whisper Base",
		290_403_536,
		true,
	),
	[ISttModelType.WhisperBaseEn]: whisperPreset(
		ISttModelType.WhisperBaseEn,
		"openai/whisper-base.en",
		"Whisper Base (English)",
		290_403_536,
		false,
	),
	[ISttModelType.WhisperSmall]: whisperPreset(
		ISttModelType.WhisperSmall,
		"openai/whisper-small",
		"Whisper Small",
		967_032_832,
		true,
	),
	[ISttModelType.WhisperSmallEn]: whisperPreset(
		ISttModelType.WhisperSmallEn,
		"openai/whisper-small.en",
		"Whisper Small (English)",
		967_032_832,
		false,
	),
	[ISttModelType.WhisperMedium]: whisperPreset(
		ISttModelType.WhisperMedium,
		"openai/whisper-medium",
		"Whisper Medium",
		3_055_544_304,
		true,
	),
	[ISttModelType.WhisperMediumEn]: whisperPreset(
		ISttModelType.WhisperMediumEn,
		"openai/whisper-medium.en",
		"Whisper Medium (English)",
		3_055_544_304,
		false,
	),
	[ISttModelType.WhisperLargeV3]: whisperPreset(
		ISttModelType.WhisperLargeV3,
		"openai/whisper-large-v3",
		"Whisper Large v3",
		3_087_284_232,
		true,
	),
	[ISttModelType.WhisperLargeV3Turbo]: whisperPreset(
		ISttModelType.WhisperLargeV3Turbo,
		"openai/whisper-large-v3-turbo",
		"Whisper Large v3 Turbo",
		1_620_000_000,
		true,
	),
	[ISttModelType.DistilWhisperMediumEn]: whisperPreset(
		ISttModelType.DistilWhisperMediumEn,
		"distil-whisper/distil-medium.en",
		"Distil-Whisper Medium (English)",
		1_516_000_000,
		false,
	),
	[ISttModelType.DistilWhisperLargeV2]: whisperPreset(
		ISttModelType.DistilWhisperLargeV2,
		"distil-whisper/distil-large-v2",
		"Distil-Whisper Large v2",
		1_512_000_000,
		true,
	),
	[ISttModelType.DistilWhisperLargeV3]: whisperPreset(
		ISttModelType.DistilWhisperLargeV3,
		"distil-whisper/distil-large-v3",
		"Distil-Whisper Large v3",
		1_512_000_000,
		true,
	),
	[ISttModelType.OlmoAsrTinyEn]: olmoPreset(
		ISttModelType.OlmoAsrTinyEn,
		"OLMoASR Tiny (English)",
		"tiny.en.pt",
		75_000_000,
	),
	[ISttModelType.OlmoAsrBaseEn]: olmoPreset(
		ISttModelType.OlmoAsrBaseEn,
		"OLMoASR Base (English)",
		"base.en.pt",
		145_000_000,
	),
	[ISttModelType.OlmoAsrSmallEn]: olmoPreset(
		ISttModelType.OlmoAsrSmallEn,
		"OLMoASR Small (English)",
		"small.en.pt",
		483_000_000,
	),
	[ISttModelType.OlmoAsrMediumEn]: olmoPreset(
		ISttModelType.OlmoAsrMediumEn,
		"OLMoASR Medium (English)",
		"medium.en.pt",
		1_530_000_000,
	),
	[ISttModelType.OlmoAsrLargeEn]: olmoPreset(
		ISttModelType.OlmoAsrLargeEn,
		"OLMoASR Large (English)",
		"large.en.pt",
		2_880_000_000,
	),
	[ISttModelType.OlmoAsrLargeEnV2]: olmoPreset(
		ISttModelType.OlmoAsrLargeEnV2,
		"OLMoASR Large v2 (English)",
		"large.en-v2.pt",
		2_880_000_000,
	),
	[ISttModelType.Qwen3Asr17B]: {
		assets: [
			asset("model.safetensors", 3_400_000_000),
			asset("vocab.json", 2_776_833),
			asset("config.json", 1_400, { required: false }),
			asset("tokenizer_config.json", 5_000, { required: false }),
			asset("preprocessor_config.json", 500, { required: false }),
			asset("merges.txt", 1_671_853, { required: false }),
			asset("generation_config.json", 300, { required: false }),
		],
		authors: ["https://huggingface.co/Qwen"],
		defaultLanguage: "auto",
		description:
			"Local Qwen3-ASR 1.7B multilingual speech-to-text model running on Candle.",
		docsUrl: "https://huggingface.co/Qwen/Qwen3-ASR-1.7B",
		label: "Qwen3-ASR 1.7B",
		languages: MULTILINGUAL_LANGS,
		license: "apache-2.0",
		longDescription:
			"Qwen3-ASR-1.7B is a multilingual transcription model loaded locally through any-speech-to-text. This preset installs the safetensors weights and tokenizer assets from Qwen/Qwen3-ASR-1.7B.",
		modelId: "Qwen/Qwen3-ASR-1.7B",
		modelType: ISttModelType.Qwen3Asr17B,
		tags: [
			"speech-to-text",
			"local",
			"any-speech-to-text",
			"qwen3-asr",
			"multilingual",
		],
	},
	[ISttModelType.MoonshineBaseEn]: {
		assets: [
			asset("config.json", 2_000),
			asset("model.safetensors", 190_000_000),
			asset("tokenizer.json", 1_500_000),
		],
		authors: ["https://huggingface.co/UsefulSensors"],
		defaultLanguage: "en",
		description:
			"Local Moonshine Base English speech-to-text model running on Candle.",
		docsUrl: "https://huggingface.co/UsefulSensors/moonshine-base",
		label: "Moonshine Base (English)",
		languages: ["en"],
		license: "mit",
		longDescription:
			"Moonshine Base is a fast English transcription model loaded locally through any-speech-to-text. This preset installs the config, safetensors weights, and tokenizer from UsefulSensors/moonshine-base.",
		modelId: "UsefulSensors/moonshine-base",
		modelType: ISttModelType.MoonshineBaseEn,
		tags: [
			"speech-to-text",
			"local",
			"any-speech-to-text",
			"moonshine",
			"english",
		],
	},
};

export function getSttModelPreset(modelType: ISttModelType): SttModelPreset {
	return (
		STT_MODEL_PRESETS[modelType] ??
		STT_MODEL_PRESETS[ISttModelType.WhisperLargeV3Turbo]
	);
}

function sttManifestFileName(modelId: string): string {
	const modelName = modelId.split("/").pop() || "stt-model";
	return `${modelName.replace(/[^a-zA-Z0-9._-]/g, "-")}.stt.json`;
}

function sttAssetRefs(assets: SttAssetDraft[]): ISttModelParameters["assets"] {
	return assets.map((asset) => ({
		bit: asset.bit.id,
		relative_path: asset.relativePath,
		required: asset.required,
	}));
}

export function defaultSttAssetLayout(
	modelType: ISttModelType,
	createBit: () => IBit,
): SttAssetDraft[] {
	const preset = getSttModelPreset(modelType);

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
						description: i18next.t('labelAssetRelativepath', '{{label}} asset: {{relativePath}}', { label: preset.label, relativePath: presetAsset.relativePath }),
						name: `${preset.label} ${presetAsset.relativePath}`,
						tags: ["stt-asset", "any-speech-to-text", ...preset.tags],
					},
				},
				repository: `https://huggingface.co/${sourceRepo}`,
				size: presetAsset.size,
			},
		};
	});
}

export function applySttModelPreset(
	bit: IBit,
	modelType: ISttModelType,
	assets: SttAssetDraft[],
): IBit {
	const preset = getSttModelPreset(modelType);
	const parameters = (bit.parameters ?? {}) as Partial<ISttModelParameters>;

	return {
		...bit,
		authors: preset.authors,
		download_link: "",
		file_name: sttManifestFileName(preset.modelId),
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
			assets: sttAssetRefs(assets),
			default_language: preset.defaultLanguage,
			dtype: ISttDTypePreference.Auto,
			languages: preset.languages,
			model_type: modelType,
			provider: {
				...parameters.provider,
				model_id: preset.modelId,
				provider_name: "local:any-speech-to-text",
				version: null,
			},
			runtime: ISttRuntimePreference.Auto,
		},
		repository: preset.docsUrl,
		size: 0,
	};
}

export function STTConfiguration({
	bit,
	setBit,
	assetBits,
	setAssetBits,
	createAssetBit,
}: Readonly<{
	bit: IBit;
	setBit: Dispatch<SetStateAction<IBit>>;
	assetBits: SttAssetDraft[];
	setAssetBits: Dispatch<SetStateAction<SttAssetDraft[]>>;
	createAssetBit: () => IBit;
}>) {
	const { t } = useTranslation("common");
	const parameters = bit.parameters as ISttModelParameters;
	const selectedModelType =
		parameters.model_type ?? ISttModelType.WhisperLargeV3Turbo;
	const selectedPreset = getSttModelPreset(selectedModelType);
	const totalAssetSize = assetBits.reduce(
		(total, asset) => total + (asset.bit.size ?? 0),
		0,
	);

	const applyPreset = (modelType: ISttModelType) => {
		const assets = defaultSttAssetLayout(modelType, createAssetBit);
		setAssetBits(assets);
		setBit((old) => applySttModelPreset(old, modelType, assets));
	};

	return (
		<div className="space-y-6 w-full max-w-screen-lg">
			<Card className="w-full">
				<CardHeader>
					<CardTitle>{t('sttModel', 'STT Model')}</CardTitle>
					<CardDescription>{t('descriptionLengthFiles', '{{description}} {{length}} files,', { description: selectedPreset.description, length: assetBits.length })}{" "}
						{humanFileSize(totalAssetSize)}.
					</CardDescription>
				</CardHeader>
				<CardContent className="space-y-2">
					<Label htmlFor="stt-model-type">{t('model', 'Model')}</Label>
					<Select
						value={selectedModelType}
						onValueChange={(value) => applyPreset(value as ISttModelType)}
					>
						<SelectTrigger id="stt-model-type">
							<SelectValue placeholder={t('selectModel', 'Select model')} />
						</SelectTrigger>
						<SelectContent>
							{STT_MODEL_TYPES.map((modelType) => {
								const preset = getSttModelPreset(modelType);
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
