export interface IEmbeddingModelParameters {
	input_length: number;
	languages: string[];
	pooling: IPooling;
	prefix: IPrefix;
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

export interface IPrefix {
	paragraph: string;
	query: string;
	[property: string]: any;
}

export interface IModelProvider {
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
