export interface IImageEmbeddingModelParameters {
	languages: string[];
	pooling: IPooling;
	provider: IModelProvider;
	remote?: null | IRemoteExecutionConfig;
	vector_length: number;
	[property: string]: any;
}

export enum IPooling {
	Cls = "CLS",
	Mean = "Mean",
	None = "None",
}

export interface IModelProvider {
	api_surface?: null | IModelApiSurface;
	model_id?: null | string;
	params?: { [key: string]: any } | null;
	provider_name: string;
	version?: null | string;
	[property: string]: any;
}

export interface IRemoteExecutionConfig {
	endpoint?: null | string;
	implementation?: null | IRemoteEmbeddingProvider;
	model_id?: null | string;
	secret_name?: null | string;
	[property: string]: any;
}

export enum IRemoteEmbeddingProvider {
	Internal = "Internal",
}

export enum IModelApiSurface {
	ChatCompletions = "ChatCompletions",
	Responses = "Responses",
}
