// ─── Data Studio query workbench types (mirror Rust workbench::* ) ───

export type QuerySurface = "native" | "overlay";
export type SavedQueryKind = "query" | "view";

export interface QueryColumn {
	name: string;
	type_name: string;
	position: number;
	metadata?: Record<string, string>;
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

/** One app's tables in a cross-app query, addressed in SQL as `<alias>.<table>`. */
export interface CrossAppSqlSource {
	alias: string;
	app_id: string;
	scope: "project" | "personal";
	/** An external data source of `app_id` instead of the app's own tables. */
	source_id?: string;
}

/** A comparison between a `$parameter` and a source column, read as `column <operator> $parameter`. */
export interface SqlParameterBinding {
	/** `alias.table` for cross-app SQL, the bare table name on a workbench surface. */
	table: string;
	column: string;
	operator: "eq" | "neq" | "lt" | "lte" | "gt" | "gte" | "like" | "ilike" | "in";
}

export interface SqlParameterInfo {
	/** Without the leading `$`. */
	name: string;
	/** Arrow type the statement fixes, e.g. `Utf8`, `Int64`, `Timestamp(µs)`. */
	data_type: string | null;
	bindings: SqlParameterBinding[];
}

export interface SqlQueryDescription {
	columns: QueryColumn[];
	parameters: SqlParameterInfo[];
}

export interface SqlParameterOptionsRequest {
	table: string;
	column: string;
	search?: string;
	/** Ask for `{min, max}` instead of distinct values. */
	range?: boolean;
}

export type SqlParameterOptions =
	| { values: unknown[]; truncated: boolean }
	| { min: unknown; max: unknown };

export interface CrossAppSqlPayload {
	sql: string;
	sources: CrossAppSqlSource[];
	/** Named `$param` values, keyed by name (no leading `$`). */
	params?: Record<string, unknown>;
	limit?: number;
	/** How long an identical request may reuse this result; 0 disables caching. */
	cache_seconds?: number;
	/** Skip any cached result and store the new one. */
	fresh?: boolean;
}

export interface CrossAppSqlResult extends ExecuteSqlResult {
	/** `alias.table` of every scanned table. */
	tables: string[];
	/** When the rows were computed, in epoch milliseconds. */
	computed_at: number;
	cached: boolean;
	/** When a cached copy stops being reused, in epoch milliseconds. */
	expires_at: number | null;
	/** Served from the cache after a refresh failed; `warnings` says why. */
	stale?: boolean;
	/** Soft limits that were passed, or limits the service lowered. */
	warnings?: string[];
}

export type ForecastTimestampFormat =
	| "auto"
	| "unix_seconds"
	| "unix_milliseconds"
	| "unix_microseconds"
	| "unix_nanoseconds"
	| "day_first"
	| "month_first";

export interface StudioForecastMapping {
	timestamp: string;
	/** Omission preserves the original integer-as-milliseconds interpretation. */
	timestamp_format?: ForecastTimestampFormat;
	series_id?: string;
	targets: string[];
	past_covariates?: string[];
	known_covariates?: string[];
	frequency_ms?: number;
}

export interface StudioForecastPayload {
	profile_id: string;
	model_bit_id: string;
	sql: string;
	params?: Record<string, unknown>;
	mapping: StudioForecastMapping;
	horizon: number;
	quantile_levels?: number[];
	future_sql?: string;
	future_params?: Record<string, unknown>;
	backtest_horizon?: number;
	/** Historical observations after the evaluated window; zero uses the final window. */
	backtest_offset?: number;
	budget?: {
		maximum_rows?: number;
		maximum_bytes?: number;
		maximum_tensor_elements?: number;
	};
}

export interface StudioHistoryPoint {
	series_id: string;
	timestamp_ms: number;
	target: string;
	value: number | null;
}

export interface StudioForecastPoint {
	series_id: string;
	timestamp_ms: number;
	target: string;
	point: number;
	/** Values follow StudioForecastResult.quantile_levels. */
	quantiles: number[];
	actual: number | null;
}

export interface StudioCovariatePoint {
	series_id: string;
	timestamp_ms: number;
	name: string;
	kind: "past" | "known";
	value: number;
	future: boolean;
}

export interface StudioBacktestMetric {
	series_id: string;
	target: string;
	observations: number;
	mae: number | null;
	rmse: number | null;
	/** Absent when an observed actual is zero; otherwise a percentage. */
	mape: number | null;
	/** Total absolute error / total absolute actuals, as a percentage. */
	wape?: number | null;
}

export type StudioForecastFamily =
	| "timestral"
	| "chronos2"
	| "toto2"
	| "timesfm2_5"
	| "tiny_time_mixer";

export interface StudioForecastResult {
	run_id: string;
	model: {
		bit_id: string;
		family: StudioForecastFamily;
		repository: string | null;
		revision: string | null;
	};
	capabilities: {
		family: StudioForecastFamily;
		max_context: number;
		max_horizon: number | null;
		native_horizon: number;
		quantile_levels: number[];
		joint_multivariate: boolean;
		past_covariates: boolean;
		known_covariates: boolean;
	};
	input_rows: number;
	horizon: number;
	quantile_levels: number[];
	point_statistic: "mean" | "median";
	history: StudioHistoryPoint[];
	forecasts: StudioForecastPoint[];
	covariates: StudioCovariatePoint[];
	backtest: {
		horizon: number;
		offset?: number;
		forecasts: StudioForecastPoint[];
		metrics: StudioBacktestMetric[];
	} | null;
}

/** A completed prediction retains the request that produced its saved result. */
export interface SavedStudioForecast {
	id: string;
	app_id: string;
	name: string;
	description?: string | null;
	created_by: string;
	created_at: string;
	request: Omit<StudioForecastPayload, "profile_id">;
	result: StudioForecastResult;
}

export interface StudioForecastSummary
	extends Omit<SavedStudioForecast, "request" | "result"> {
	model_bit_id: string;
	horizon: number;
	run_id: string;
}

export interface CreateStudioForecastPayload {
	name: string;
	description?: string | null;
	request: SavedStudioForecast["request"];
	result: StudioForecastResult;
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
	extends Omit<
		Partial<CreateSavedQueryPayload>,
		| "description"
		| "overlay_id"
		| "param_schema"
		| "viz_config"
		| "default_limit"
	> {
	description?: string | null;
	overlay_id?: string | null;
	param_schema?: Record<string, unknown> | null;
	viz_config?: VizConfig | null;
	default_limit?: number | null;
	/** Required optimistic-concurrency token: the `updated_at` last loaded. */
	expected_updated_at: string;
}

export interface IQueryState {
	listForecasts(
		appId: string,
		userScoped?: boolean,
		maximum?: number,
	): Promise<StudioForecastSummary[]>;
	getForecast(
		appId: string,
		forecastId: string,
		userScoped?: boolean,
	): Promise<SavedStudioForecast>;
	createForecast(
		appId: string,
		payload: CreateStudioForecastPayload,
		userScoped?: boolean,
	): Promise<SavedStudioForecast>;
	deleteForecast(
		appId: string,
		forecastId: string,
		userScoped?: boolean,
	): Promise<void>;
	forecast(
		appId: string,
		payload: StudioForecastPayload,
		userScoped?: boolean,
	): Promise<StudioForecastResult>;
	executeSql(
		appId: string,
		payload: ExecuteSqlPayload,
		userScoped?: boolean,
	): Promise<ExecuteSqlResult>;
	/** One read-only statement over the tables of several apps, with server-side caching. */
	executeCrossAppSql(payload: CrossAppSqlPayload): Promise<CrossAppSqlResult>;
	/** Output columns and `$parameters` of cross-app SQL, without running it. */
	describeCrossAppSql(
		payload: Pick<CrossAppSqlPayload, "sql" | "sources">,
	): Promise<SqlQueryDescription>;
	/** Values (or the range) a cross-app `$parameter` bound to `table.column` can take. */
	crossAppSqlParameterOptions(
		payload: Pick<CrossAppSqlPayload, "sources"> & SqlParameterOptionsRequest,
	): Promise<SqlParameterOptions>;
	/** Output columns and `$parameters` of workbench SQL, without running it. */
	describeSql(
		appId: string,
		payload: Pick<ExecuteSqlPayload, "sql" | "surface" | "overlay_id">,
		userScoped?: boolean,
	): Promise<SqlQueryDescription>;
	sqlParameterOptions(
		appId: string,
		payload: Pick<ExecuteSqlPayload, "surface" | "overlay_id"> &
			SqlParameterOptionsRequest,
		userScoped?: boolean,
	): Promise<SqlParameterOptions>;
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
