export interface FlowLikeClientOptions {
	baseUrl?: string;
	pat?: string;
	apiKey?: string;
}

export interface AuthConfig {
	type: "pat" | "api_key";
	token: string;
}

export interface TriggerOptions {
	headers?: Record<string, string>;
	signal?: AbortSignal;
	timeout?: number;
}

export interface InvokeBoardRequest {
	node_id: string;
	version?: [number, number, number];
	payload?: unknown;
	token?: string;
	oauth_tokens?: Record<string, unknown>;
	stream_state?: boolean;
	runtime_variables?: Record<string, unknown>;
	profile_id?: string;
}

export interface WorkflowTriggerOptions
	extends TriggerOptions,
		InvokeBoardQuery {
	version?: Version;
	token?: string;
	oauth_tokens?: JsonObject;
	runtime_variables?: JsonObject;
	profile_id?: string;
	stream_state?: boolean;
}

export interface InvokeBoardQuery {
	local?: boolean;
	isolated?: boolean;
}

export interface AsyncInvokeResult {
	run_id: string;
	status: string;
	poll_token: string;
	backend?: string;
}

export interface SSEEvent {
	event?: string;
	data: string;
	id?: string;
}

export type JsonObject = Record<string, unknown>;
export type VersionType = "Major" | "Minor" | "Patch";
export type Version = [number, number, number];
export interface FileScope {
	scope?: "project" | "user";
	signal?: AbortSignal;
}
export interface ListFilesOptions extends FileScope {
	prefix?: string;
	refresh?: boolean;
}
export interface FileEntry {
	location: string;
	size: number;
	last_modified: string;
	e_tag: string | null;
	version: string | null;
	is_dir: boolean;
}
export type ListFilesResult = FileEntry[];
export interface UploadFileOptions extends FileScope {
	key?: string;
	contentType?: string;
}
export interface DownloadFileOptions extends FileScope {}
export interface SignedFile {
	prefix: string;
	url?: string;
	error?: string;
	method?: "PUT" | "POST";
	fields?: Record<string, string>;
}
export interface PresignOptions extends FileScope {
	prefix?: string;
	access_mode?: "read" | "write";
}
export interface PresignResult {
	shared_credentials: SharedCredentials;
	path: string;
	access_mode: string;
	expiration?: string | null;
}

export interface BucketConfig {
	endpoint?: string;
	express?: boolean;
}

export interface AwsSharedCredentials {
	access_key_id?: string;
	secret_access_key?: string;
	session_token?: string;
	meta_bucket: string;
	content_bucket: string;
	logs_bucket: string;
	meta_config?: BucketConfig;
	content_config?: BucketConfig;
	logs_config?: BucketConfig;
	region: string;
	expiration?: string;
	content_path_prefix?: string;
	user_content_path_prefix?: string;
}

export interface AzureSharedCredentials {
	meta_sas_token?: string;
	content_sas_token?: string;
	user_content_sas_token?: string;
	logs_sas_token?: string;
	meta_container: string;
	content_container: string;
	logs_container: string;
	account_name: string;
	account_key?: string;
	expiration?: string;
	content_path_prefix?: string;
	user_content_path_prefix?: string;
}

export interface GcpSharedCredentials {
	service_account_key: string;
	access_token?: string;
	meta_bucket: string;
	content_bucket: string;
	logs_bucket: string;
	allowed_prefixes: string[];
	write_access: boolean;
	expiration?: string;
	content_path_prefix?: string;
	user_content_path_prefix?: string;
}

export type SharedCredentials =
	| { Aws: AwsSharedCredentials }
	| { Azure: AzureSharedCredentials }
	| { Gcp: GcpSharedCredentials }
	| {
			Mixed: {
				meta: SharedCredentials;
				content: SharedCredentials;
				logs: SharedCredentials;
			};
	  };

export interface PresignDbAccessResponse {
	shared_credentials: SharedCredentials;
	db_path: string;
	table_name: string;
	access_mode: string;
	expiration?: string;
}

export interface LanceConnectionInfo {
	uri: string;
	storageOptions: Record<string, string>;
}

/** Arrow's serialized schema. Field data types may be scalar names or nested type objects. */
export interface TableSchema {
	fields: TableField[];
	metadata?: Record<string, string>;
}
export interface TableField {
	name: string;
	data_type: unknown;
	nullable: boolean;
	metadata?: Record<string, string>;
	[key: string]: unknown;
}
export interface EventTriggerOptions extends TriggerOptions {
	version?: string;
	token?: string;
	oauth_tokens?: JsonObject;
	runtime_variables?: JsonObject;
	profile_id?: string;
	correlation?: Record<string, string>;
	page_trigger?: JsonObject;
	local?: boolean;
	isolated?: boolean;
	variant?: string;
}

export interface DatabaseOptions {
	scope?: "project" | "user";
	branch?: string;
	version?: number;
	tag?: string;
	read_only?: boolean;
}
export interface QueryOptions extends DatabaseOptions {
	filter?: string;
	select?: string[];
	limit?: number;
	offset?: number;
	sql?: string;
	sql_params?: unknown;
	vector_query?: { vector: number[] };
	fts_term?: string;
	rerank?: boolean;
}
export type CountResult = number;
export interface RunStatus {
	run_id: string;
	board_id: string;
	event_id: string | null;
	status: string;
	mode: string;
	progress: number;
	current_step: string | null;
	error: string | null;
	input_payload_len: number;
	output_payload_len: number;
	started_at: string | null;
	completed_at: string | null;
	created_at: string;
}
export interface PollOptions {
	afterSequence?: number;
	timeout?: number;
	signal?: AbortSignal;
}
export interface PollResult {
	run_id: string;
	status: string;
	progress: number;
	current_step: string | null;
	error: string | null;
	events: PollEvent[];
	started_at: string | null;
	completed_at: string | null;
	/** Highest returned sequence, or the supplied cursor when this page is empty. */
	lastSequence: number;
}
export interface PollEvent {
	sequence: number;
	event_type: string;
	payload: unknown;
	created_at: string;
}
export interface RunListOptions {
	node_id?: string;
	from?: number;
	to?: number;
	status?: number;
	limit?: number;
	offset?: number;
	include_nodes?: boolean;
}
export interface LogQuery {
	levels?: number[];
	exclude_levels?: number[];
	nodes?: string[];
	exclude_nodes?: string[];
	text?: string[];
	exclude_text?: string[];
	fingerprints?: string[];
	exclude_fingerprints?: string[];
	from?: number;
	to?: number;
	fold?: { fingerprint: string; first_start: number }[];
}
export interface RunLogOptions {
	query?: LogQuery;
	limit?: number;
	offset?: number;
}
export type DatabaseAction =
	| {
			action:
				| "create_branch"
				| "delete_branch"
				| "create_tag"
				| "update_tag"
				| "delete_tag"
				| "clone";
			name: string;
	  }
	| { action: "restore" }
	| { action: "snapshot"; name?: string }
	| { action: "cleanup"; older_than_days: number };
export interface CreateAppOptions {
	meta: JsonObject;
	bits?: string[];
	language?: string;
}
export interface EventUpsertOptions {
	version_type?: VersionType;
	pat?: string;
	oauth_tokens?: JsonObject;
	profile_id?: string;
	register_source?: boolean;
}

export interface HttpSinkOptions {
	headers?: Record<string, string>;
	signal?: AbortSignal;
}

export interface ChatMessage {
	role: "system" | "user" | "assistant" | "function";
	content: string;
	name?: string;
}

export interface ChatCompletionOptions {
	temperature?: number;
	max_tokens?: number;
	top_p?: number;
	stream?: boolean;
	stop?: string | string[];
	tools?: unknown[];
	signal?: AbortSignal;
}

export interface ChatCompletionResult {
	[key: string]: unknown;
}

/** Which upstream API a model Bit speaks. Unset means Chat Completions. */
export type ModelApiSurface = "ChatCompletions" | "Responses";

export interface ResponsesOptions {
	instructions?: string;
	temperature?: number;
	max_output_tokens?: number;
	top_p?: number;
	tools?: unknown[];
	signal?: AbortSignal;
}

export interface ResponsesResult {
	[key: string]: unknown;
}

export interface EmbeddingTextInput {
	type: "text";
	text: string;
}

export interface EmbeddingImageInput {
	type: "image";
	/** HTTP(S) URL, base64 data URL, or raw base64. */
	image: string;
}

export interface EmbeddingAudioInput {
	type: "audio";
	/** HTTP(S) URL, base64 data URL, or raw base64. */
	audio: string;
}

export interface EmbeddingVideoInput {
	type: "video";
	/** HTTP(S) URL, base64 data URL, or raw base64. */
	video: string;
}

/** Combines text and up to one file per modality into one embedding. */
export interface EmbeddingMultimodalInput {
	type: "multimodal";
	text?: string;
	image?: string;
	audio?: string;
	video?: string;
}

export type EmbeddingInput =
	| string
	| EmbeddingTextInput
	| EmbeddingImageInput
	| EmbeddingAudioInput
	| EmbeddingVideoInput
	| EmbeddingMultimodalInput;

export interface EmbedOptions {
	embed_type?: "query" | "document";
	signal?: AbortSignal;
}

export interface EmbedResult {
	embeddings: number[][];
	model: string;
	usage: { prompt_tokens: number; total_tokens: number };
	/** True when token counts are estimated from input bytes. */
	usage_estimated?: boolean;
	/** Whether the provider reported token usage. Older servers omit this field. */
	usage_available?: boolean;
}

export type BitType =
	| "Llm"
	| "Vlm"
	| "Embedding"
	| "ImageEmbedding"
	| "File"
	| "Media"
	| "Template"
	| "ObjectDetection"
	| "Other";

export interface BitMetadata {
	name: string;
	description: string;
	long_description?: string;
	tags: string[];
	icon?: string;
}

export interface Bit {
	id: string;
	type: BitType;
	meta: Record<string, BitMetadata>;
	authors: string[];
	repository?: string;
	parameters: unknown;
	version?: string;
	license?: string;
	hub: string;
	[key: string]: unknown;
}

export interface BitSearchQuery {
	search?: string;
	limit?: number;
	offset?: number;
	bit_types?: BitType[];
}

export interface ModelInfo {
	bit_id: string;
	name: string;
	description: string;
	provider_name?: string;
	api_surface?: ModelApiSurface;
	model_id?: string;
	context_length?: number;
	vector_length?: number;
	languages?: string[];
	tags: string[];
}

export interface ChatUsage {
	llm_price: number;
	embedding_price: number;
}

export interface UpsertBoardRequest {
	name?: string;
	description?: string;
	stage?: string;
	log_level?: string;
	execution_mode?: string;
	template?: unknown;
}

export interface UpsertBoardResponse {
	id: string;
	updated_at: { secs_since_epoch: number; nanos_since_epoch: number };
	board?: Board | null;
}

export interface Board {
	id: string;
	[key: string]: unknown;
}

export interface PrerunBoardResponse {
	runtime_variables: unknown[];
	oauth_requirements: unknown[];
	requires_local_execution: boolean;
	execution_mode: string;
	can_execute_locally: boolean;
}

export interface App {
	id: string;
	name?: string;
	[key: string]: unknown;
}

export interface HealthResult {
	status: string;
	[key: string]: unknown;
}
