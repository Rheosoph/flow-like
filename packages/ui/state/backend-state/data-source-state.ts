// ─── External data sources (mirror Rust routes/app/data_sources) ───

export type ExternalDataSourceDriver =
	| "postgres"
	| "mysql"
	| "clickhouse"
	| "bigquery"
	| "athena";

/** An external database an app's queries can read as `<alias>.<table>`. Secrets never leave the hub. */
export interface ExternalDataSource {
	id: string;
	app_id: string;
	name: string;
	/** Schema name in SQL; lowercase letters, digits and _, unique per app. */
	alias: string;
	driver: ExternalDataSourceDriver;
	/** Non-secret connection settings (host, port, database, schema, ssl_mode, project_id, …). */
	config: Record<string, unknown>;
	/** Names of the stored secret fields (e.g. `username`, `password`), never their values. */
	secret_fields: string[];
	created_by: string;
	created_at: string;
	updated_at: string;
	last_checked_at?: string | null;
	/** `ok`, or the error of the last connection check. */
	last_check_status?: string | null;
}

export interface UpsertExternalDataSourcePayload {
	name: string;
	alias: string;
	driver: ExternalDataSourceDriver;
	config: Record<string, unknown>;
	/** Omit on update to keep the stored secret; send `{}` to clear it. */
	secret?: Record<string, string>;
}

export interface ExternalDataSourceCheck {
	ok: boolean;
	tables: string[];
	error?: string | null;
}

export interface IDataSourceState {
	listDataSources(appId: string): Promise<ExternalDataSource[]>;
	getDataSource(appId: string, sourceId: string): Promise<ExternalDataSource>;
	createDataSource(
		appId: string,
		payload: UpsertExternalDataSourcePayload,
	): Promise<ExternalDataSource>;
	updateDataSource(
		appId: string,
		sourceId: string,
		payload: Partial<UpsertExternalDataSourcePayload>,
	): Promise<ExternalDataSource>;
	deleteDataSource(appId: string, sourceId: string): Promise<void>;
	/** Connects with the stored secret and lists the tables queries can address. */
	checkDataSource(
		appId: string,
		sourceId: string,
	): Promise<ExternalDataSourceCheck>;
}
