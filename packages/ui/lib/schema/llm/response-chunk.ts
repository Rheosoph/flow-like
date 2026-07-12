import type { IContent } from "./history";

export interface IResponseChunk {
	choices: IResponseChunkChoice[];
	created?: number | null;
	id: string;
	model?: null | string;
	service_tier?: null | string;
	system_fingerprint?: null | string;
	usage?: null | IUsage;
	x_prefill_progress?: number | null;
	[property: string]: any;
}

export interface IResponseChunkChoice {
	delta?: null | IDelta;
	finish_reason?: null | string;
	index: number;
	logprobs?: null | ILogProbs;
	[property: string]: any;
}

export interface IDelta {
	content?: null | string;
	content_parts?: IContent[] | null;
	reasoning?: null | string;
	refusal?: null | string;
	role?: null | string;
	tool_calls?: IFunctionCall[] | null;
	[property: string]: any;
}

export interface IFunctionCall {
	function: IResponseFunction;
	id: string;
	index?: number | null;
	type?: null | string;
	[property: string]: any;
}

export interface IResponseFunction {
	arguments?: null | string;
	name?: null | string;
	[property: string]: any;
}

export interface ILogProbs {
	content?: ITokenLogProbs[] | null;
	refusal?: ITokenLogProbs[] | null;
	[property: string]: any;
}

export interface ITokenLogProbs {
	bytes?: number[] | null;
	logprob: number;
	token: string;
	top_logprobs?: ITopLogProbs[] | null;
	[property: string]: any;
}

export interface ITopLogProbs {
	bytes?: number[] | null;
	logprob: number;
	token: string;
	[property: string]: any;
}

export interface IUsage {
	completion_tokens: number;
	completion_tokens_details?: null | ICompletionTokenDetails;
	prompt_tokens: number;
	prompt_tokens_details?: null | IPromptTokenDetails;
	total_tokens: number;
	[property: string]: any;
}

export interface ICompletionTokenDetails {
	accepted_prediction_tokens: number;
	audio_tokens: number;
	reasoning_tokens: number;
	rejected_prediction_tokens: number;
	[property: string]: any;
}

export interface IPromptTokenDetails {
	audio_tokens: number;
	cached_tokens: number;
	[property: string]: any;
}
