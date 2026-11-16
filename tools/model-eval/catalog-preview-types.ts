export type PreviewKind = "image" | "video" | "audio";
export interface CatalogPreviewAsset {
	url: string;
	kind: PreviewKind;
	source_url: string;
	description: string;
	scope?: "output" | "base_model_output" | "input_demo";
}
export interface CatalogPreviewEntry {
	bit_ids: string[];
	model_id?: string;
	repository?: string;
	media: CatalogPreviewAsset[];
}
