export interface IEmbeddingModelParameters {
    input_length: number;
    languages:    string[];
    pooling:      IPooling;
    prefix:       IPrefix;
    provider:     IProvider;
    /**
     * Remote execution configuration (for API proxy mode)
     */
    remote?:       null | IRemoteObject;
    vector_length: number;
    [property: string]: any;
}

export enum IPooling {
    Cls = "CLS",
    Mean = "Mean",
    None = "None",
}

export interface IPrefix {
    paragraph: string;
    query:     string;
    [property: string]: any;
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

/**
 * Configuration for remote execution via API proxy
 */
export interface IRemoteObject {
    /**
     * Legacy endpoint URL retained for reading old Bits. Hosted execution
     * resolves endpoints from server secrets instead of using this URL.
     */
    endpoint?: null | string;
    /**
     * Optional server secret containing the base endpoint. Supported by
     * Internal, AzureOpenAI, HuggingfaceEndpoint and OpenAICompatible.
     */
    endpoint_secret_name?: null | string;
    /**
     * Which remote provider implementation to use
     */
    implementation?: null | string;
    /**
     * Deployed model ID for the upstream provider (e.g., Internal gateway model name)
     */
    model_id?: null | string;
    /**
     * Optional API key secret override. Each provider has its own default.
     */
    secret_name?: null | string;
    /**
     * Whether OpenRouter accepts a requested output dimension count. Set false
     * for fixed-width models. When absent, the Bit's vector length is requested.
     */
    supports_dimensions?: boolean | null;
    [property: string]: any;
}
