export interface IBitSearchQuery {
	bit_types?: IBitTypes[] | null;
	limit?: number | null;
	offset?: number | null;
	search?: null | string;
	[property: string]: any;
}

export enum IBitTypes {
	Board = "Board",
	Config = "Config",
	Course = "Course",
	Embedding = "Embedding",
	File = "File",
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
	Template = "Template",
	Tokenizer = "Tokenizer",
	TokenizerConfig = "TokenizerConfig",
	Stt = "Stt",
	Tts = "Tts",
	Vlm = "Vlm",
	VideoGeneration = "VideoGeneration",
}
