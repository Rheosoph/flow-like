export interface IResponse {
    choices:             IChoiceElement[];
    created?:            number | null;
    id?:                 null | string;
    model?:              null | string;
    object?:             null | string;
    service_tier?:       null | string;
    system_fingerprint?: null | string;
    usage:               IUsage;
    [property: string]: any;
}

export interface IChoiceElement {
    finish_reason: string;
    index:         number;
    logprobs?:     null | ILogprobsObject;
    message:       IMessage;
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

export interface IMessage {
    annotations?: IAnnotationElement[] | null;
    audio?:       null | IAudioObject;
    content?:     null | string;
    /**
     * Ordered structured content returned by Rig when the assistant response contains media.
     * Text-only responses continue to use `content` alone for OpenAI compatibility.
     */
    content_parts?: IContentPartElement[];
    reasoning?:     null | string;
    refusal?:       null | string;
    role:           string;
    tool_calls:     IToolCallElement[];
    [property: string]: any;
}

export interface IAnnotationElement {
    type:          string;
    url_citation?: null | IURLCitationObject;
    [property: string]: any;
}

export interface IURLCitationObject {
    content?:    null | string;
    end_index:   number;
    start_index: number;
    title:       string;
    url:         string;
    [property: string]: any;
}

export interface IAudioObject {
    data:        string;
    expires_at?: number | null;
    id:          string;
    transcript?: null | string;
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
    id:       string;
    index?:   number | null;
    type?:    null | string;
    [property: string]: any;
}

export interface IFunction {
    arguments: string;
    name:      string;
    [property: string]: any;
}

export interface IUsage {
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
