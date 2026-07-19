export interface IHistory {
	frequency_penalty?: number | null;
	max_completion_tokens?: number | null;
	messages: IHistoryMessage[];
	model: string;
	n?: number | null;
	presence_penalty?: number | null;
	response_format?: any;
	seed?: number | null;
	stop?: string[] | null;
	stream?: boolean | null;
	stream_options?: null | IStreamOptions;
	temperature?: number | null;
	tool_choice?: null | IToolChoice;
	tools?: ITool[] | null;
	top_p?: number | null;
	user?: null | string;
	[property: string]: any;
}

export interface IHistoryMessage {
	content: IContent[] | string;
	name?: null | string;
	role: IRole;
	tool_call_id?: null | string;
	tool_calls?: IToolCall[] | null;
	[property: string]: any;
}

export interface IContent {
	text?: string;
	type: IContentType;
	image_url?: IImageURL;
	audio_url?: string;
	video_url?: string;
	document_url?: string;
	media_type?: string;
	additional_params?: unknown;
	[property: string]: any;
}

export interface IImageURL {
	detail?: null | string;
	media_type?: string;
	additional_params?: unknown;
	url: string;
	[property: string]: any;
}

export enum IContentType {
	IImageURL = "image_url",
	AudioURL = "audio_url",
	VideoURL = "video_url",
	DocumentURL = "document_url",
	Text = "text",
}

export enum IRole {
	Assistant = "assistant",
	Function = "function",
	System = "system",
	ITool = "tool",
	User = "user",
}

export interface IToolCall {
	function: IToolCallFunction;
	id: string;
	type: string;
	[property: string]: any;
}

export interface IToolCallFunction {
	arguments?: null | string;
	name?: null | string;
	[property: string]: any;
}

export interface IStreamOptions {
	include_usage: boolean;
	[property: string]: any;
}

export interface IToolChoice {
	function: IHistoryFunction;
	type: IToolType;
	[property: string]: any;
}

export interface IHistoryFunction {
	description?: null | string;
	name: string;
	parameters: IHistoryFunctionParameters;
	[property: string]: any;
}

export interface IHistoryFunctionParameters {
	properties?: { [key: string]: IHistoryJSONSchemaDefine } | null;
	required?: string[] | null;
	type: IHistoryJSONSchemaType;
	[property: string]: any;
}

export interface IHistoryJSONSchemaDefine {
	description?: null | string;
	enum_values?: string[] | null;
	items?: null | IHistoryJSONSchemaDefine;
	properties?: { [key: string]: IHistoryJSONSchemaDefine } | null;
	required?: string[] | null;
	type?: IHistoryJSONSchemaType | null;
	[property: string]: any;
}

export enum IHistoryJSONSchemaType {
	Array = "array",
	Boolean = "boolean",
	Null = "null",
	Number = "number",
	Object = "object",
	String = "string",
}

export enum IToolType {
	Function = "function",
}

export interface ITool {
	function: IHistoryFunction;
	type: IToolType;
	[property: string]: any;
}
