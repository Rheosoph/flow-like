export enum IIndexType {
	FullText = 0,
	BTree = 1,
	Bitmap = 2,
	LabelList = 3,
	Auto = 4,
	Vector = 5,
	Fm = 6,
	IvfFlat = 7,
	IvfPq = 8,
	IvfSq = 9,
	IvfRq = 10,
	IvfHnswFlat = 11,
	IvfHnswPq = 12,
	IvfHnswSq = 13,
	NGram = 14,
	ZoneMap = 15,
	BloomFilter = 16,
	RTree = 17,
}

const INDEX_TYPE_NAMES: Record<IIndexType, string> = {
	[IIndexType.FullText]: "FullText",
	[IIndexType.BTree]: "BTree",
	[IIndexType.Bitmap]: "Bitmap",
	[IIndexType.LabelList]: "LabelList",
	[IIndexType.Auto]: "Auto",
	[IIndexType.Vector]: "Vector",
	[IIndexType.Fm]: "Fm",
	[IIndexType.IvfFlat]: "IvfFlat",
	[IIndexType.IvfPq]: "IvfPq",
	[IIndexType.IvfSq]: "IvfSq",
	[IIndexType.IvfRq]: "IvfRq",
	[IIndexType.IvfHnswFlat]: "IvfHnswFlat",
	[IIndexType.IvfHnswPq]: "IvfHnswPq",
	[IIndexType.IvfHnswSq]: "IvfHnswSq",
	[IIndexType.NGram]: "NGram",
	[IIndexType.ZoneMap]: "ZoneMap",
	[IIndexType.BloomFilter]: "BloomFilter",
	[IIndexType.RTree]: "RTree",
};

/** Use the same index names for HTTP requests and desktop commands. */
export function indexTypeToString(indexType: IIndexType): string {
	return INDEX_TYPE_NAMES[indexType] ?? "Auto";
}

/** Accept persisted enum values, API names, and node option labels. */
export function parseIndexType(value: unknown): IIndexType {
	if (typeof value === "number") {
		return Object.hasOwn(INDEX_TYPE_NAMES, value) ? value : IIndexType.Auto;
	}
	const normalized = String(value ?? "Auto")
		.replace(/[\s_-]/g, "")
		.toLowerCase();
	if (normalized === "fts" || normalized === "inverted") {
		return IIndexType.FullText;
	}
	for (const [type, name] of Object.entries(INDEX_TYPE_NAMES)) {
		if (name.toLowerCase() === normalized) return Number(type) as IIndexType;
	}
	return IIndexType.Auto;
}

export interface IQueryTableVectorPayload {
	column: string;
	vector: number[];
}

export interface IQueryTablePayload {
	sql?: string;
	/** Values for `sql`'s `$placeholders`, keyed by placeholder name without the `$`. */
	sql_params?: Record<string, unknown>;
	/** Columns to return; omitted returns every column. */
	select?: string[];
	vector_query?: IQueryTableVectorPayload;
	filter?: string;
	fts_term?: string;
	rerank?: boolean;
}

export interface IIndexConfig {
	name: string;
	index_type: string;
	columns: string[];
}

/** Exactly one of `sql_expression` or `type`. */
export interface IAddColumnPayload {
	name: string;
	/** Computes the column from existing columns. */
	sql_expression?: string;
	/** A table column type such as `geometry`; the column starts empty. */
	type?: string;
	vector_size?: number;
}

export interface IDatabaseSchemaField {
	name: string;
	type: string;
	nullable?: boolean;
	vector_size?: number;
	/** Marks the table key; at most one required field of a key type. */
	primary_key?: boolean;
}

export interface ICreateTableResult {
	table_name: string;
	created: boolean;
	if_not_exists: boolean;
}

export interface IDropTableResult {
	table_name: string;
	dropped: boolean;
	ontologies: string[];
	saved_queries: string[];
	warnings: string[];
}

/** Coarse type bucket the backend classifies each column into. */
export type IColumnFamily =
	| "text"
	| "number"
	| "time"
	| "bool"
	| "vector"
	| "struct"
	| "geo"
	| "binary"
	| "other";

export interface IColumnSummary {
	name: string;
	data_type: string;
	family: IColumnFamily;
	nullable: boolean;
	vector_size?: number;
}

export interface IIndexSummary {
	name: string;
	index_type: string;
	columns: string[];
}

/** Absent when LanceDB could not report fragment statistics for the table. */
export interface IStorageSummary {
	total_bytes: number;
	num_fragments: number;
	/** Fragments below the compaction threshold — a high count means `optimize` is worth running. */
	num_small_fragments: number;
}

/** What the semantic layer and the query workbench do with a table. */
export interface IConsumerSummary {
	ontology?: string;
	ontology_id?: string;
	object_type?: string;
	object_color?: string;
	object_icon?: string;
	relations: number;
	actions: number;
	views: number;
	queries: number;
	exposed: boolean;
}

export interface ITableSummary {
	name: string;
	rows?: number;
	columns: IColumnSummary[];
	indexes: IIndexSummary[];
	storage?: IStorageSummary;
	consumers: IConsumerSummary;
	/** Set when this one table failed to read; the rest of the listing still resolved. */
	error?: string;
}

/** A table reference. Versions and tags are read-only snapshots. */
export interface IDatabaseSelector {
	branch?: string;
	version?: number;
	tag?: string;
	read_only?: boolean;
}

export interface IDatabaseReference {
	table: string;
	branch: string;
	version: number;
	read_only: boolean;
	pinned: boolean;
}

export interface IDatabaseVersion {
	version: number;
	timestamp: string;
	metadata: Record<string, string>;
}

export interface IDatabaseBranch {
	name: string;
	parent_branch?: string;
	parent_version?: number;
	/** Unix timestamp in seconds. */
	created_at?: number;
}

export interface IDatabaseTag {
	name: string;
	branch: string;
	version: number;
	created_at?: string;
	updated_at?: string;
}

export interface IDatabaseDiff {
	source: IDatabaseReference;
	target: IDatabaseReference;
	added: number;
	removed: number;
	changed: number;
	unchanged: number;
	schema_changes: string[];
	rows: {
		kind: "added" | "removed" | "changed";
		key: unknown;
		before?: unknown;
		after?: unknown;
	}[];
	truncated: boolean;
}

export interface IDatabaseHistory {
	reference: IDatabaseReference;
	versions: IDatabaseVersion[];
	branches: IDatabaseBranch[];
	tags: IDatabaseTag[];
}

export interface IDatabaseCleanupStats {
	bytes_removed: number;
	old_versions: number;
	data_files_removed: number;
	transaction_files_removed: number;
	index_files_removed: number;
	deletion_files_removed: number;
}

export type IDatabaseAction =
	| {
			action:
				| "create_branch"
				| "delete_branch"
				| "create_tag"
				| "update_tag"
				| "delete_tag";
			name: string;
	  }
	| { action: "clone"; name: string }
	| { action: "restore" }
	| { action: "snapshot"; name?: string }
	| { action: "cleanup"; older_than_days: number };

export interface IDatabaseActionResult {
	reference: IDatabaseReference;
	cleanup?: IDatabaseCleanupStats;
}

/** Serialize selectors consistently for hosted web and desktop requests. */
export function databaseQueryParams(
	userScoped?: boolean,
	selector?: IDatabaseSelector,
): URLSearchParams {
	const params = new URLSearchParams();
	if (userScoped) params.set("scope", "user");
	if (selector?.branch !== undefined) params.set("branch", selector.branch);
	if (selector?.version !== undefined)
		params.set("version", String(selector.version));
	if (selector?.tag !== undefined) params.set("tag", selector.tag);
	if (selector?.read_only !== undefined)
		params.set("read_only", String(selector.read_only));
	return params;
}

export interface IDatabaseState {
	databaseCompare(
		appId: string,
		tableName: string,
		otherSelector: IDatabaseSelector,
		key: string,
		limit?: number,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<IDatabaseDiff>;
	databaseHistory(
		appId: string,
		tableName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<IDatabaseHistory>;
	databaseAction(
		appId: string,
		tableName: string,
		action: IDatabaseAction,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<IDatabaseActionResult>;
	createTable(
		appId: string,
		tableName: string,
		fields: IDatabaseSchemaField[],
		ifNotExists?: boolean,
		userScoped?: boolean,
	): Promise<ICreateTableResult>;
	buildIndex(
		appId: string,
		tableName: string,
		column: string,
		indexType: IIndexType,
		optimize?: boolean,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void>;
	addItems(
		appId: string,
		tableName: string,
		items: any[],
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void>;
	removeItems(
		appId: string,
		tableName: string,
		query: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void>;
	listItems(
		appId: string,
		tableName: string,
		offset?: number,
		limit?: number,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<any[]>;
	queryItems(
		appId: string,
		tableName: string,
		query: IQueryTablePayload,
		offset?: number,
		limit?: number,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<any[]>;
	countItems(
		appId: string,
		tableName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<number>;
	/** {@link listItems} that propagates read failures instead of answering an empty page. */
	listItemsAuthoritative?(
		appId: string,
		tableName: string,
		offset?: number,
		limit?: number,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<unknown[]>;
	/** {@link queryItems} that propagates read failures instead of answering no rows. */
	queryItemsAuthoritative?(
		appId: string,
		tableName: string,
		query: IQueryTablePayload,
		offset?: number,
		limit?: number,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<unknown[]>;
	/** {@link countItems} that propagates read failures instead of answering 0. */
	countItemsAuthoritative?(
		appId: string,
		tableName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<number>;
	getSchema(
		appId: string,
		tableName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<any>;
	/** Read the schema from one explicit authority without cache or routing fallback. */
	getSchemaAuthoritative(
		appId: string,
		tableName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<any>;
	getIndices(
		appId: string,
		tableName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<IIndexConfig[]>;
	dropIndex(
		appId: string,
		tableName: string,
		indexName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void>;
	listTables(appId: string): Promise<string[]>;
	/** List tables from one explicit authority and propagate read failures. */
	listTablesAuthoritative(appId: string): Promise<string[]>;
	listTablesUser(appId: string): Promise<string[]>;
	/**
	 * One metadata-only pass over every table: rows, schema, indexes, storage
	 * footprint and the ontology objects, actions and saved queries that read it.
	 * Costs more than {@link listTables} — use it only where the detail is shown.
	 */
	listTableSummaries(
		appId: string,
		userScoped?: boolean,
	): Promise<ITableSummary[]>;
	optimize(
		appId: string,
		tableName: string,
		keepVersions?: boolean,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void>;
	updateItem(
		appId: string,
		tableName: string,
		filter: string,
		updates: Record<string, any>,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void>;
	dropColumns(
		appId: string,
		tableName: string,
		columns: string[],
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void>;
	addColumn(
		appId: string,
		tableName: string,
		column: IAddColumnPayload,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void>;
	alterColumn(
		appId: string,
		tableName: string,
		column: string,
		nullable: boolean,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void>;
	/**
	 * Mark `column` as the table key (Lance's unenforced primary key) so
	 * concurrent Upserts on it cannot insert duplicates. A key is permanent.
	 */
	setPrimaryKey(
		appId: string,
		tableName: string,
		column: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void>;
	dropTable(
		appId: string,
		tableName: string,
		userScoped?: boolean,
	): Promise<IDropTableResult>;
}
