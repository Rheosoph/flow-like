export interface IBitPack {
	bits: IBit[];
	[property: string]: any;
}

export interface IBit {
	authors: string[];
	created: string;
	dependencies: string[];
	dependency_tree_hash: string;
	download_link?: null | string;
	file_name?: null | string;
	hash: string;
	hub: string;
	id: string;
	license?: null | string;
	meta: { [key: string]: IMetadata };
	parameters: any;
	repository?: null | string;
	size?: number | null;
	type: IBitTypes;
	updated: string;
	version?: null | string;
	[property: string]: any;
}

export interface IMetadata {
	age_rating?: number | null;
	created_at: ISystemTime;
	description: string;
	docs_url?: null | string;
	icon?: null | string;
	long_description?: null | string;
	name: string;
	organization_specific_values?: number[] | null;
	preview_media: string[];
	release_notes?: null | string;
	support_url?: null | string;
	tags: string[];
	thumbnail?: null | string;
	updated_at: ISystemTime;
	use_case?: null | string;
	website?: null | string;
	[property: string]: any;
}

export interface ISystemTime {
	nanos_since_epoch: number;
	secs_since_epoch: number;
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
