export interface IResponseChunk {
    choices:             IChoiceElement[];
    created?:            number | null;
    id:                  string;
    model?:              null | string;
    service_tier?:       null | string;
    system_fingerprint?: null | string;
    usage?:              null | IUsageObject;
    x_prefill_progress?: number | null;
    [property: string]: any;
}

export interface IChoiceElement {
    delta?:         null | IDeltaObject;
    finish_reason?: null | string;
    index:          number;
    logprobs?:      null | ILogprobsObject;
    [property: string]: any;
}

export interface IDeltaObject {
    content?:       null | string;
    content_parts?: IContentPartElement[] | null;
    reasoning?:     null | string;
    refusal?:       null | string;
    role?:          null | string;
    tool_calls?:    IToolCallElement[] | null;
    [property: string]: any;
}

export interface IContentPartElement {
    text?:              string;
    type:               IType;
    image_url?:         IImageURL;
    additional_params?: any;
    audio_url?:         string;
    media_type?:        null | string;
    video_url?:         string;
    document_url?:      string;
    [property: string]: any;
}

export interface IImageURL {
    additional_params?: any;
    detail?:            null | string;
    media_type?:        null | string;
    url:                string;
    [property: string]: any;
}

export enum IType {
    AudioURL = "audio_url",
    DocumentURL = "document_url",
    IImageURL = "image_url",
    Text = "text",
    VideoURL = "video_url",
}

export interface IToolCallElement {
    function: IFunction;
    id?:      null | string;
    index?:   number | null;
    type?:    null | string;
    [property: string]: any;
}

export interface IFunction {
    arguments?: null | string;
    name?:      null | string;
    [property: string]: any;
}

export interface ILogprobsObject {
    content?: IContentElement[] | null;
    refusal?: IContentElement[] | null;
    [property: string]: any;
}

export interface IContentElement {
    bytes?:        number[] | null;
    logprob:       number;
    token:         string;
    top_logprobs?: ITopLogprobElement[] | null;
    [property: string]: any;
}

export interface ITopLogprobElement {
    bytes?:  number[] | null;
    logprob: number;
    token:   string;
    [property: string]: any;
}

export interface IUsageObject {
    completion_tokens:          number;
    completion_tokens_details?: null | ICompletionTokensDetailsObject;
    cost?:                      number | null;
    prompt_tokens:              number;
    prompt_tokens_details?:     null | IPromptTokensDetailsObject;
    total_tokens:               number;
    upstream_inference_cost?:   null | IUpstreamInferenceCostObject;
    [property: string]: any;
}

export interface ICompletionTokensDetailsObject {
    accepted_prediction_tokens?: number | null;
    audio_tokens?:               number | null;
    reasoning_tokens?:           number | null;
    rejected_prediction_tokens?: number | null;
    [property: string]: any;
}

export interface IPromptTokensDetailsObject {
    audio_tokens?:  number | null;
    cached_tokens?: number | null;
    [property: string]: any;
}

export interface IUpstreamInferenceCostObject {
    upstream_inference_cost?: number | null;
    [property: string]: any;
}
