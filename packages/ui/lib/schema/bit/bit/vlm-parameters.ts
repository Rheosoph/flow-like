export interface IVlmParameters {
	context_length: number;
	/**
	 * The most sensitive data this model may process; unset means untagged.
	 */
	data_classification?: null | IDataClassification;
	model_classification: IBitModelClassification;
	provider: IModelProvider;
	[property: string]: any;
}

/**
 * How sensitive data is, from least to most sensitive.
 */
export type IDataClassification =
	| "public"
	| "internal"
	| "confidential"
	| "restricted";

export interface IBitModelClassification {
	coding: number;
	cost: number;
	creativity: number;
	factuality: number;
	function_calling: number;
	multilinguality: number;
	openness: number;
	reasoning: number;
	safety: number;
	speed: number;
	[property: string]: any;
}

export interface IModelProvider {
	api_surface?: null | IModelApiSurface;
	model_id?: null | string;
	provider_name: string;
	version?: null | string;
	[property: string]: any;
}

export enum IModelApiSurface {
	ChatCompletions = "ChatCompletions",
	Responses = "Responses",
	SystemOne = "SystemOne",
}
