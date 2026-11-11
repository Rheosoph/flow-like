export interface IProfile {
    apps?:   IAppElement[] | null;
    bits:    string[];
    created: string;
    /**
     * User-owned private bits. Only the owner's desktop and the hub hold their
     * provider secrets. Server runs carry secret-free views (provider and
     * model, no key or endpoint) and call them through the hub's model relay;
     * the browser client and the server profile row never see a secret.
     * Schema-wise these are plain `Bit`s (the wrapper is serde-transparent);
     * `schemars(with)` keeps the generated schema referencing `Bit` instead
     * of minting a duplicate inline type, which would cascade renames through
     * the quicktype-generated TS.
     */
    custom_bits?: ICustomBitElement[];
    description?: null | string;
    /**
     * Bots this profile leaves out of its crew, pickers and Home. A custom bot goes by its id,
     * a built-in one by its system key, so the choice holds for its account and device rows.
     * Hiding is a view filter: the bot keeps running and stays reachable through its chats.
     */
    hidden_bots?:     string[];
    home_default_id?: null | string;
    home_layout?:     any;
    hub?:             string;
    hubs?:            string[];
    icon?:            null | string;
    id?:              string;
    interests?:       string[];
    name:             string;
    secure?:          boolean;
    settings?:        ISettings;
    shortcuts?:       IShortcutElement[] | null;
    tags?:            string[];
    theme?:           any;
    thumbnail?:       null | string;
    updated:          string;
    [property: string]: any;
}

export interface IAppElement {
    app_id:          string;
    favorite:        boolean;
    favorite_order?: number | null;
    pinned:          boolean;
    pinned_order?:   number | null;
    [property: string]: any;
}

export interface ICustomBitElement {
    authors?:              string[];
    created?:              string;
    dependencies?:         string[];
    dependency_tree_hash?: string;
    download_link?:        null | string;
    file_name?:            null | string;
    hash?:                 string;
    hub?:                  string;
    id?:                   string;
    license?:              null | string;
    meta?:                 { [key: string]: IMetaValue };
    model_evaluation?:     null | IModelEvaluationObject;
    model_slug?:           null | string;
    parameters?:           any;
    repository?:           null | string;
    size?:                 number | null;
    type?:                 IType;
    updated?:              string;
    version?:              null | string;
    [property: string]: any;
}

export interface IMetaValue {
    age_rating?:                   number | null;
    created_at:                    ICreatedAt;
    description:                   string;
    docs_url?:                     null | string;
    icon?:                         null | string;
    long_description?:             null | string;
    name:                          string;
    organization_specific_values?: number[] | null;
    preview_media:                 string[];
    release_notes?:                null | string;
    support_url?:                  null | string;
    tags:                          string[];
    thumbnail?:                    null | string;
    updated_at:                    ICreatedAt;
    use_case?:                     null | string;
    website?:                      null | string;
    [property: string]: any;
}

export interface ICreatedAt {
    nanos_since_epoch: number;
    secs_since_epoch:  number;
    [property: string]: any;
}

export interface IModelEvaluationObject {
    creator_name:                        string;
    creator_slug:                        string;
    evaluations?:                        any;
    median_output_tokens_per_second?:    number | null;
    median_time_to_first_answer_token?:  number | null;
    median_time_to_first_token_seconds?: number | null;
    name:                                string;
    pricing?:                            any;
    release_date?:                       null | string;
    slug:                                string;
    [property: string]: any;
}

export enum IType {
    Board = "Board",
    Config = "Config",
    Course = "Course",
    Embedding = "Embedding",
    File = "File",
    Forecasting = "Forecasting",
    ImageEmbedding = "ImageEmbedding",
    ImageGeneration = "ImageGeneration",
    Llm = "Llm",
    Media = "Media",
    ObjectDetection = "ObjectDetection",
    Other = "Other",
    PreprocessorConfig = "PreprocessorConfig",
    Project = "Project",
    Projection = "Projection",
    SpecialTokensMap = "SpecialTokensMap",
    Stt = "Stt",
    SystemOne = "SystemOne",
    TTS = "Tts",
    Template = "Template",
    Tokenizer = "Tokenizer",
    TokenizerConfig = "TokenizerConfig",
    VideoGeneration = "VideoGeneration",
    Vlm = "Vlm",
}

export interface ISettings {
    connection_mode: IConnectionMode;
    [property: string]: any;
}

export enum IConnectionMode {
    Default = "default",
    Simplebezier = "simplebezier",
    Smoothstep = "smoothstep",
    Step = "step",
    Straight = "straight",
}

export interface IShortcutElement {
    appId?:    null | string;
    createdAt: string;
    icon?:     null | string;
    id:        string;
    label:     string;
    order:     number;
    path:      string;
    profileId: string;
    [property: string]: any;
}
