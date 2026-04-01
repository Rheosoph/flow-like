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

export interface ListFilesOptions {
	prefix?: string;
	cursor?: string;
	limit?: number;
}

export interface FileEntry {
	key: string;
	size: number;
	lastModified: string;
	contentType?: string;
}

export interface ListFilesResult {
	files: FileEntry[];
	cursor?: string;
	hasMore: boolean;
}

export interface UploadFileOptions {
	key?: string;
	contentType?: string;
}

export interface DownloadFileOptions {
	signal?: AbortSignal;
}

export interface PresignOptions {
	key: string;
	method?: "GET" | "PUT";
	expiresIn?: number;
}

export interface PresignResult {
	url: string;
	expiresAt: string;
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
	| { Gcp: GcpSharedCredentials };

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

export interface TableSchema {
	name: string;
	fields: TableField[];
}

export interface TableField {
	name: string;
	type: string;
	nullable: boolean;
}

export interface QueryOptions {
	filter?: string;
	select?: string[];
	limit?: number;
	offset?: number;
}

export interface CountResult {
	count: number;
}

export interface RunStatus {
	runId: string;
	status: string;
	result?: unknown;
	error?: string;
	createdAt?: string;
	updatedAt?: string;
}

export interface PollOptions {
	afterSequence?: number;
	timeout?: number;
	signal?: AbortSignal;
}

export interface PollResult {
	events: PollEvent[];
	lastSequence: number;
}

export interface PollEvent {
	sequence: number;
	type: string;
	data: unknown;
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

export interface ChatUsage {
	promptTokens: number;
	completionTokens: number;
	totalTokens: number;
}

export interface EmbedOptions {
	embed_type?: "query" | "document";
	signal?: AbortSignal;
}

export interface EmbedResult {
	embeddings: number[][];
	model: string;
	usage: { prompt_tokens: number; total_tokens: number };
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
	name: string;
	[key: string]: unknown;
}

export interface HealthResult {
	status: string;
	[key: string]: unknown;
}
