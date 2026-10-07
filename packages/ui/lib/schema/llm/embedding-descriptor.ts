export interface IEmbeddingDescriptor {
	adapter: string;
	/**
	 * Exact sets of modalities supported in a joint item. Single modalities use `modalities`.
	 */
	joint_combinations: Array<IModalityElement[]>;
	limits: ILimits;
	modalities: IModalityElement[];
	model_id: string;
	/**
	 * Hash of resolved weights, tokenizer, preprocessing, and output configuration.
	 */
	pipeline_fingerprint: string;
	purposes: IPurposeElement[];
	space: ISpace;
	supported_dimensions: number[];
	[property: string]: any;
}

export enum IModalityElement {
	Audio = "audio",
	Image = "image",
	Text = "text",
	Video = "video",
}

export interface ILimits {
	max_audio_duration_ms?: number | null;
	max_batch_items?: number | null;
	max_images_per_item?: number | null;
	max_tokens: number;
	max_video_frames?: number | null;
	[property: string]: any;
}

export enum IPurposeElement {
	Classification = "classification",
	Clustering = "clustering",
	CodeQuery = "code_query",
	Document = "document",
	Query = "query",
	Similarity = "similarity",
}

export interface ISpace {
	dimensions: number;
	/**
	 * Identifies a validated compatible query/document space, including the output recipe.
	 */
	id: string;
	metric: IMetric;
	normalized: boolean;
	[property: string]: any;
}

export enum IMetric {
	Cosine = "cosine",
	DotProduct = "dot_product",
	Euclidean = "euclidean",
}
