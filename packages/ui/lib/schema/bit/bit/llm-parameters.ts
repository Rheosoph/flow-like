export interface ILlmParameters {
    context_length: number;
    /**
     * The most sensitive data this model may process; `None` means untagged.
     */
    data_classification?: null | string;
    model_classification: IModelClassification;
    provider:             IProvider;
    [property: string]: any;
}

export interface IModelClassification {
    coding:           number;
    cost:             number;
    creativity:       number;
    factuality:       number;
    function_calling: number;
    multilinguality:  number;
    openness:         number;
    reasoning:        number;
    safety:           number;
    speed:            number;
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
