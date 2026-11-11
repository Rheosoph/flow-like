import type { IModelProvider } from "./llm-parameters";

export type ISystemOneParameters = {
	context_length: number;
	provider: IModelProvider;
	[property: string]: unknown;
};
