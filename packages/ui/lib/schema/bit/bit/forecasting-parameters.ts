/**
 * Checkpoint architecture and files for a forecasting model Bit.
 */
export interface IForecastingParameters {
    assets?:    IAssetElement[];
    model_type: IModelType;
    provider?:  IProvider;
    [property: string]: any;
}

/**
 * A required checkpoint file supplied by a dependency Bit.
 */
export interface IAssetElement {
    bit:           string;
    relative_path: string;
    [property: string]: any;
}

/**
 * Native architecture used to read a forecasting checkpoint.
 */
export enum IModelType {
    Chronos2 = "chronos2",
    Timesfm25 = "timesfm2_5",
    Timestral = "timestral",
    TinyTimeMixer = "tiny_time_mixer",
    Toto2 = "toto2",
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
