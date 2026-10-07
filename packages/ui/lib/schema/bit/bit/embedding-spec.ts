/**
 * Versioned model recipe stored in a Bit's `parameters.embedding` field.
 */
export interface IEmbeddingSpec {
	adapter: string;
	adapter_version?: number;
	artifacts: { [key: string]: IArtifactValue };
	dimensions: number;
	max_tokens: number;
	pooling?: IPooling;
	schema_version: number;
	space_id: string;
	supported_dimensions?: number[];
	task_profiles?: ITaskProfiles;
	[property: string]: any;
}

/**
 * A Bit dependency assigned to a model role and its graph-relative destination.
 */
export interface IArtifactValue {
	bit: string;
	path: string;
	source?: null | ISourceObject;
	[property: string]: any;
}

/**
 * A downloadable artifact whose bytes are pinned by a BLAKE3 digest.
 */
export interface ISourceObject {
	hash: string;
	size: number;
	url: string;
	[property: string]: any;
}

export enum IPooling {
	Cls = "cls",
	LastToken = "last_token",
	Mean = "mean",
}

export interface ITaskProfiles {
	classification?: IClassification;
	clustering?: IClassification;
	code_query?: IClassification;
	document?: IClassification;
	query?: IClassification;
	similarity?: IClassification;
}

export interface IClassification {
	prefix?: string;
	suffix?: string;
	[property: string]: any;
}
