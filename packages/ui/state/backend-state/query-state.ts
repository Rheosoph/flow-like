// ─── Data Studio query workbench types (mirror Rust workbench::* ) ───

export type QuerySurface = "native" | "overlay";
export type SavedQueryKind = "query" | "view";

export interface QueryColumn {
	name: string;
	type_name: string;
	position: number;
}

export interface ExecuteSqlPayload {
	sql: string;
	/** Named `$param` values, keyed by name (no leading `$`). */
	params?: Record<string, unknown>;
	surface: QuerySurface;
	overlay_id?: string;
	limit?: number;
}

export interface ExecuteSqlResult {
	columns: QueryColumn[];
	rows: Record<string, unknown>[];
	row_count: number;
	truncated: boolean;
}

// ─── Result visualization config (persisted with a saved query) ───

export type VizView = "table" | "chart" | "graph" | "json";
export type ChartType = "bar" | "line" | "area" | "pie" | "scatter";

export interface VizChartConfig {
	type: ChartType;
	/** Category / x-axis column. */
	x?: string;
	/** Value / y-axis column(s). */
	y?: string[];
	/** Optional grouping / series column. */
	series?: string;
}

export interface GraphVizConfig {
	source?: string;
	target?: string;
	label?: string;
	weight?: string;
}

export interface VizConfig {
	view?: VizView;
	chart?: VizChartConfig;
	graph?: GraphVizConfig;
}

// ─── Saved query artifact (query or view) ───

export interface SavedQuery {
	id: string;
	app_id: string;
	name: string;
	description?: string;
	kind: SavedQueryKind;
	surface: QuerySurface;
	overlay_id?: string;
	sql: string;
	/** JSON-Schema-shaped parameter definition (properties/required). */
	param_schema?: Record<string, unknown>;
	viz_config?: VizConfig;
	default_limit?: number;
	created_at: string;
	updated_at: string;
}

export interface CreateSavedQueryPayload {
	name: string;
	description?: string;
	kind: SavedQueryKind;
	surface: QuerySurface;
	overlay_id?: string;
	sql: string;
	param_schema?: Record<string, unknown>;
	viz_config?: VizConfig;
	default_limit?: number;
}

export interface UpdateSavedQueryPayload
	extends Partial<CreateSavedQueryPayload> {
	/** Optimistic-concurrency token: the `updated_at` last loaded. */
	expected_updated_at?: string;
}

export interface IQueryState {
	executeSql(
		appId: string,
		payload: ExecuteSqlPayload,
		userScoped?: boolean,
	): Promise<ExecuteSqlResult>;
	listSavedQueries(appId: string, userScoped?: boolean): Promise<SavedQuery[]>;
	getSavedQuery(
		appId: string,
		queryId: string,
		userScoped?: boolean,
	): Promise<SavedQuery>;
	createSavedQuery(
		appId: string,
		payload: CreateSavedQueryPayload,
		userScoped?: boolean,
	): Promise<SavedQuery>;
	updateSavedQuery(
		appId: string,
		queryId: string,
		payload: UpdateSavedQueryPayload,
		userScoped?: boolean,
	): Promise<SavedQuery>;
	deleteSavedQuery(
		appId: string,
		queryId: string,
		userScoped?: boolean,
	): Promise<void>;
}
