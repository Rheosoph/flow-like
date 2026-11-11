export interface IBitSearchQuery {
    bit_types?: IBitTypeElement[] | null;
    limit?:     number | null;
    offset?:    number | null;
    search?:    null | string;
    [property: string]: any;
}

export enum IBitTypeElement {
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
