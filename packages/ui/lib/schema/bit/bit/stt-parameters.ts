import type { IModelProvider } from "./llm-parameters";

export interface ISttModelParameters {
	assets: ISttAssetRef[];
	default_language?: null | string;
	dtype?: ISttDTypePreference | null;
	languages: string[];
	model_type: ISttModelType;
	provider: IModelProvider;
	runtime?: ISttRuntimePreference | null;
	[property: string]: any;
}

export interface ISttAssetRef {
	bit: string;
	relative_path: string;
	required: boolean;
	[property: string]: any;
}

export enum ISttDTypePreference {
	Auto = "Auto",
	BF16 = "BF16",
	F16 = "F16",
	F32 = "F32",
}

export enum ISttModelType {
	DistilWhisperLargeV2 = "DistilWhisperLargeV2",
	DistilWhisperLargeV3 = "DistilWhisperLargeV3",
	DistilWhisperMediumEn = "DistilWhisperMediumEn",
	MoonshineBaseEn = "MoonshineBaseEn",
	OlmoAsrBaseEn = "OlmoAsrBaseEn",
	OlmoAsrLargeEn = "OlmoAsrLargeEn",
	OlmoAsrLargeEnV2 = "OlmoAsrLargeEnV2",
	OlmoAsrMediumEn = "OlmoAsrMediumEn",
	OlmoAsrSmallEn = "OlmoAsrSmallEn",
	OlmoAsrTinyEn = "OlmoAsrTinyEn",
	Qwen3Asr17B = "Qwen3Asr17B",
	WhisperBase = "WhisperBase",
	WhisperBaseEn = "WhisperBaseEn",
	WhisperLargeV3 = "WhisperLargeV3",
	WhisperLargeV3Turbo = "WhisperLargeV3Turbo",
	WhisperMedium = "WhisperMedium",
	WhisperMediumEn = "WhisperMediumEn",
	WhisperSmall = "WhisperSmall",
	WhisperSmallEn = "WhisperSmallEn",
	WhisperTiny = "WhisperTiny",
	WhisperTinyEn = "WhisperTinyEn",
}

export enum ISttRuntimePreference {
	Accelerate = "Accelerate",
	Auto = "Auto",
	Cpu = "Cpu",
	Cuda = "Cuda",
	Metal = "Metal",
}
