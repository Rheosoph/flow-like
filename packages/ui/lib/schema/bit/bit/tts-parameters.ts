export interface ITTSParameters {
    assets?:           IAssetElement[];
    default_language?: null | string;
    default_voice?:    null | string;
    dtype?:            IDtypeEnum | null;
    languages?:        string[];
    model_type?:       IModelType;
    provider?:         IProvider;
    runtime?:          IRuntimeEnum | null;
    voices?:           string[];
    [property: string]: any;
}

export interface IAssetElement {
    bit:           string;
    relative_path: string;
    required?:     boolean;
    [property: string]: any;
}

export enum IDtypeEnum {
    Auto = "Auto",
    Bf16 = "BF16",
    F16 = "F16",
    F32 = "F32",
}

export enum IModelType {
    Kokoro = "Kokoro",
    OmniVoice = "OmniVoice",
    Qwen3TTS = "Qwen3Tts",
    VibeVoice = "VibeVoice",
    VibeVoiceRealtime = "VibeVoiceRealtime",
    Voxtral = "Voxtral",
}

export interface IProvider {
    /**
     * API surface the upstream model speaks. `None` keeps the provider's
     * historical chat default. SystemOne Bits always use the decision API.
     */
    api_surface?:  IAPISurfaceEnum | null;
    model_id?:     null | string;
    params?:       { [key: string]: any } | null;
    provider_name: string;
    version?:      null | string;
    [property: string]: any;
}

/**
 * Which HTTP API surface a model speaks.
 *
 * Providers historically expose the OpenAI-compatible `chat/completions`
 * shape. Newer OpenAI-family models are only reachable through the Responses
 * API (`/responses`), which uses a different request and event schema. A Bit
 * declares the surface so the client builds the matching Rig client and the
 * Flow-Like proxy relays to the matching upstream endpoint. SystemOne Bits
 * select the native decision surface through their Bit type.
 */
export enum IAPISurfaceEnum {
    ChatCompletions = "ChatCompletions",
    Responses = "Responses",
    SystemOne = "SystemOne",
}

export enum IRuntimeEnum {
    Accelerate = "Accelerate",
    Auto = "Auto",
    CPU = "Cpu",
    Cuda = "Cuda",
    Metal = "Metal",
}
