export interface IBit {
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
