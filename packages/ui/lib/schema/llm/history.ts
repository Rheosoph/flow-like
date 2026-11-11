export interface IHistory {
    frequency_penalty?:     number | null;
    max_completion_tokens?: number | null;
    messages:               IMessageElement[];
    model:                  string;
    n?:                     number | null;
    presence_penalty?:      number | null;
    preset?:                null | string;
    response_format?:       any;
    seed?:                  number | null;
    stop?:                  string[] | null;
    stream?:                boolean | null;
    stream_options?:        null | IStreamOptionsObject;
    temperature?:           number | null;
    thinking?:              IThinkingEnum | null;
    tool_choice?:           null | IToolChoiceObject;
    tools?:                 IToolElement[] | null;
    top_p?:                 number | null;
    usage?:                 null | IUsageObject;
    user?:                  null | string;
    [property: string]: any;
}

export interface IMessageElement {
    annotations?:  IAnnotationElement[] | null;
    content:       IContentElement[] | string;
    name?:         null | string;
    role:          IRole;
    tool_call_id?: null | string;
    tool_calls?:   IToolCallElement[] | null;
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

export interface IContentElement {
    text?:              string;
    type:               IContentType;
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

export enum IContentType {
    AudioURL = "audio_url",
    DocumentURL = "document_url",
    IImageURL = "image_url",
    Text = "text",
    VideoURL = "video_url",
}

export enum IRole {
    Assistant = "assistant",
    Function = "function",
    System = "system",
    Tool = "tool",
    User = "user",
}

export interface IToolCallElement {
    function: IToolCallFunction;
    id:       string;
    type:     string;
    [property: string]: any;
}

export interface IToolCallFunction {
    arguments: string;
    name:      string;
    [property: string]: any;
}

export interface IStreamOptionsObject {
    include_usage: boolean;
    [property: string]: any;
}

export enum IThinkingEnum {
    High = "high",
    Low = "low",
    Mid = "mid",
    Off = "off",
}

export interface IToolChoiceObject {
    function: IToolChoiceFunction;
    type:     IToolChoiceType;
    [property: string]: any;
}

export interface IToolChoiceFunction {
    description?: null | string;
    name:         string;
    parameters:   IParameters;
    [property: string]: any;
}

export interface IParameters {
    properties?: { [key: string]: IPropertyValue } | null;
    required?:   string[] | null;
    type:        ITypeEnum;
    [property: string]: any;
}

export interface IPropertyValue {
    default?:     any;
    description?: null | string;
    enum?:        string[] | null;
    items?:       null | IPropertyValue;
    properties?:  { [key: string]: IPropertyValue } | null;
    required?:    string[] | null;
    type?:        ITypeEnum | null;
    [property: string]: any;
}

export enum ITypeEnum {
    Array = "array",
    Boolean = "boolean",
    Null = "null",
    Number = "number",
    Object = "object",
    String = "string",
}

export enum IToolChoiceType {
    Function = "function",
}

export interface IToolElement {
    function: IToolChoiceFunction;
    type:     IToolChoiceType;
    [property: string]: any;
}

export interface IUsageObject {
    include: boolean;
    [property: string]: any;
}
