import type { IModelProvider } from "./llm-parameters";

export interface ITtsModelParameters {
	assets: ITtsAssetRef[];
	default_language?: null | string;
	default_voice?: null | string;
	dtype?: ITtsDTypePreference | null;
	languages: string[];
	model_type: ITtsModelType;
	provider: IModelProvider;
	runtime?: ITtsRuntimePreference | null;
	voices: string[];
	[property: string]: any;
}

export interface ITtsAssetRef {
	bit: string;
	relative_path: string;
	required: boolean;
	[property: string]: any;
}

export enum ITtsDTypePreference {
	Auto = "Auto",
	BF16 = "BF16",
	F16 = "F16",
	F32 = "F32",
}

export enum ITtsModelType {
	Kokoro = "Kokoro",
	OmniVoice = "OmniVoice",
	Qwen3Tts = "Qwen3Tts",
	VibeVoice = "VibeVoice",
	VibeVoiceRealtime = "VibeVoiceRealtime",
	Voxtral = "Voxtral",
}

export enum ITtsRuntimePreference {
	Accelerate = "Accelerate",
	Auto = "Auto",
	Cpu = "Cpu",
	Cuda = "Cuda",
	Metal = "Metal",
}
