export interface IEmbeddingBatch {
	embeddings: Array<number[]>;
	provenance: IProvenance;
	space: ISpace;
	usage: IUsage;
	[property: string]: any;
}

export interface IProvenance {
	adapter: string;
	model_id: string;
	pipeline_fingerprint: string;
	[property: string]: any;
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

export interface IUsage {
	audio_duration_ms: number;
	input_items: number;
	text_tokens?: number | null;
	video_frames: number;
	[property: string]: any;
}
