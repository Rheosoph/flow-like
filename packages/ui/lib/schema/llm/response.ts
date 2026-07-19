import type { IContent } from "./history";

export interface IResponse {
	choices: IChoice[];
	created?: number | null;
	id?: null | string;
	model?: null | string;
	object?: null | string;
	service_tier?: null | string;
	system_fingerprint?: null | string;
	usage: IUsage;
	[property: string]: any;
}

export interface IChoice {
	finish_reason: string;
	index: number;
	logprobs?: null | ILogProbs;
	message: IResponseMessage;
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

export interface IResponseMessage {
	annotations?: IAnnotation[] | null;
	audio?: null | IAudio;
	content?: null | string;
	content_parts?: IContent[];
	reasoning?: null | string;
	refusal?: null | string;
	role: string;
	tool_calls?: IFunctionCall[];
	[property: string]: any;
}

export interface IAnnotation {
	type: string;
	url_citation?: null | IURLCitation;
	[property: string]: any;
}

export interface IURLCitation {
	end_index: number;
	start_index: number;
	title: string;
	url: string;
	[property: string]: any;
}

export interface IAudio {
	data: string;
	expires_at?: number | null;
	id: string;
	transcript?: null | string;
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
