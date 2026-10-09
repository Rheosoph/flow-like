/** Common LanceDB operations. Import /lancedb for the peer package's full types. */
export interface LanceConnection {
	isOpen(): boolean;
	close(): void;
	display(): string;
	tableNames(options?: { startAfter?: string; limit?: number }): Promise<
		string[]
	>;
	openTable(
		name: string,
		namespace?: string[],
		options?: {
			storageOptions?: Record<string, string>;
			indexCacheSize?: number;
		},
	): Promise<LanceTable>;
	createTable(
		name: string,
		data: Record<string, unknown>[],
		options?: {
			mode?: "create" | "overwrite";
			existOk?: boolean;
			storageOptions?: Record<string, string>;
		},
	): Promise<LanceTable>;
	dropTable(name: string, namespace?: string[]): Promise<void>;
}

export interface LanceTable {
	readonly name: string;
	isOpen(): boolean;
	close(): void;
	countRows(filter?: string): Promise<number>;
	search(query: string | number[]): LanceQuery;
	vectorSearch(vector: number[]): LanceQuery;
}

export interface LanceQuery {
	where(predicate: string): LanceQuery;
	limit(limit: number): LanceQuery;
	toArray(): Promise<Record<string, unknown>[]>;
}

/** Common model operations. Import /langchain for LangChain's full Runnable types. */
export interface FlowLikeChat {
	invoke(
		input: string,
		options?: { signal?: AbortSignal },
	): Promise<{ content: unknown }>;
	batch(
		inputs: string[],
		options?: { signal?: AbortSignal },
	): Promise<{ content: unknown }[]>;
	stream(
		input: string,
		options?: { signal?: AbortSignal },
	): Promise<AsyncIterable<{ content: unknown }>>;
}

export interface FlowLikeEmbeddingModel {
	embedDocuments(texts: string[]): Promise<number[][]>;
	embedQuery(text: string): Promise<number[]>;
}

export interface FlowLikeEmbeddingsParams {
	baseUrl: string;
	token: string;
	bitId: string;
}

export interface FlowLikeChatModelParams extends FlowLikeEmbeddingsParams {
	temperature?: number;
	maxTokens?: number;
	topP?: number;
	stop?: string[];
}
