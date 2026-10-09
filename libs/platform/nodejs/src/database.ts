import { appPath, segment } from "./paths.js";
import type { DatabaseOptions, DatabaseAction, JsonObject } from "./types.js";
import type { LanceConnection } from "./integrations.js";
import type { HttpClient } from "./client.js";
import type {
	CountResult,
	LanceConnectionInfo,
	PresignDbAccessResponse,
	QueryOptions,
	TableSchema,
} from "./types.js";

function resolveConnectionInfo(
	resp: PresignDbAccessResponse,
): LanceConnectionInfo {
	let creds = resp.shared_credentials;
	while ("Mixed" in creds) creds = creds.Mixed.content;

	if ("Aws" in creds) {
		const aws = creds.Aws;
		const uri = `s3://${aws.content_bucket}/${resp.db_path}`;
		const opts: Record<string, string> = {};
		if (aws.access_key_id) opts.aws_access_key_id = aws.access_key_id;
		if (aws.secret_access_key)
			opts.aws_secret_access_key = aws.secret_access_key;
		if (aws.session_token) opts.aws_session_token = aws.session_token;
		if (aws.region) opts.aws_region = aws.region;
		if (aws.content_config?.endpoint)
			opts.aws_endpoint = aws.content_config.endpoint;
		return { uri, storageOptions: opts };
	}

	if ("Azure" in creds) {
		const az = creds.Azure;
		const uri = `az://${az.content_container}/${resp.db_path}`;
		const opts: Record<string, string> = {
			azure_storage_account_name: az.account_name,
		};
		const sas = resp.db_path.startsWith("users/")
			? (az.user_content_sas_token ?? az.content_sas_token)
			: az.content_sas_token;
		if (sas) opts.azure_storage_sas_token = sas;
		if (az.account_key) opts.azure_storage_account_key = az.account_key;
		return { uri, storageOptions: opts };
	}

	if ("Gcp" in creds) {
		const gcp = creds.Gcp;
		const uri = `gs://${gcp.content_bucket}/${resp.db_path}`;
		const opts: Record<string, string> = {};
		if (gcp.access_token) opts.google_storage_token = gcp.access_token;
		else if (gcp.service_account_key)
			opts.google_service_account_key = gcp.service_account_key;
		return { uri, storageOptions: opts };
	}

	throw new Error("Unknown shared credentials provider");
}

export function createDatabaseMethods(http: HttpClient) {
	return {
		async getDbCredentials(
			appId: string,
			tableName = "_default",
			accessMode: "read" | "write" = "read",
			scope: "user" | "project" = "user",
		): Promise<LanceConnectionInfo> {
			const resp = await http.request<PresignDbAccessResponse>(
				"POST",
				`${appPath(appId)}/db/presign${scope === "project" ? "/project" : ""}`,
				{ body: { table_name: tableName, access_mode: accessMode } },
			);
			return resolveConnectionInfo(resp);
		},

		async getDbCredentialsRaw(
			appId: string,
			tableName = "_default",
			accessMode: "read" | "write" = "read",
			scope: "user" | "project" = "user",
		): Promise<PresignDbAccessResponse> {
			return http.request<PresignDbAccessResponse>(
				"POST",
				`${appPath(appId)}/db/presign${scope === "project" ? "/project" : ""}`,
				{ body: { table_name: tableName, access_mode: accessMode } },
			);
		},

		async createLanceConnection(
			appId: string,
			accessMode: "read" | "write" = "read",
			scope: "user" | "project" = "user",
		): Promise<LanceConnection> {
			let lancedb: typeof import("@lancedb/lancedb");
			try {
				lancedb = await import("@lancedb/lancedb");
			} catch {
				throw new Error(
					"@lancedb/lancedb is required for createLanceConnection. Install it with: npm install @lancedb/lancedb",
				);
			}

			const info = await this.getDbCredentials(
				appId,
				"_default",
				accessMode,
				scope,
			);
			return lancedb.connect(info.uri, {
				storageOptions: info.storageOptions,
			});
		},

		listTables(
			appId: string,
			options: { scope?: "project" | "user" } = {},
		): Promise<string[]> {
			return http.request(
				"GET",
				`${appPath(appId)}/db${options.scope === "user" ? "/user" : ""}`,
			);
		},
		listTableSummaries(
			appId: string,
			options: { scope?: "project" | "user" } = {},
		): Promise<JsonObject[]> {
			return http.request(
				"GET",
				`${appPath(appId)}/db${options.scope === "user" ? "/user" : ""}`,
				{ query: { detail: "summary" } },
			);
		},
		getTableSchema(
			appId: string,
			table: string,
			options: DatabaseOptions = {},
		): Promise<TableSchema> {
			return http.request(
				"GET",
				`${appPath(appId)}/db/${segment(table)}/schema`,
				{ query: { ...options } },
			);
		},
		queryTable(
			appId: string,
			table: string,
			query: QueryOptions,
		): Promise<unknown[]> {
			const { limit, offset, scope, branch, version, tag, read_only, ...body } =
				query;
			return http.request(
				"POST",
				`${appPath(appId)}/db/${segment(table)}/query`,
				{
					body,
					query: { limit, offset, scope, branch, version, tag, read_only },
				},
			);
		},
		listTableItems(
			appId: string,
			table: string,
			options: DatabaseOptions & { limit?: number; offset?: number } = {},
		): Promise<unknown[]> {
			return http.request("GET", `${appPath(appId)}/db/${segment(table)}`, {
				query: { ...options },
			});
		},
		async addToTable(
			appId: string,
			table: string,
			items: unknown[],
			options: DatabaseOptions = {},
		): Promise<void> {
			await http.request("PUT", `${appPath(appId)}/db/${segment(table)}`, {
				body: { items },
				query: { ...options },
			});
		},
		async deleteFromTable(
			appId: string,
			table: string,
			filter: string,
			options: DatabaseOptions = {},
		): Promise<void> {
			await http.request("DELETE", `${appPath(appId)}/db/${segment(table)}`, {
				body: { query: filter },
				query: { ...options },
			});
		},
		countItems(
			appId: string,
			table: string,
			options: DatabaseOptions = {},
		): Promise<CountResult> {
			return http.request(
				"GET",
				`${appPath(appId)}/db/${segment(table)}/count`,
				{ query: { ...options } },
			);
		},
		createTable(
			appId: string,
			table: string,
			body: {
				fields: {
					name: string;
					data_type: string;
					nullable?: boolean;
					vector_size?: number;
					primary_key?: boolean;
				}[];
				if_not_exists?: boolean;
			},
			options: DatabaseOptions = {},
		): Promise<JsonObject> {
			return http.request("POST", `${appPath(appId)}/db/${segment(table)}`, {
				body,
				query: { ...options },
			});
		},
		async dropTable(
			appId: string,
			table: string,
			options: DatabaseOptions = {},
		): Promise<void> {
			await http.request(
				"DELETE",
				`${appPath(appId)}/db/${segment(table)}/table`,
				{ query: { ...options } },
			);
		},
		async updateTable(
			appId: string,
			table: string,
			filter: string,
			updates: JsonObject,
			options: DatabaseOptions = {},
		): Promise<void> {
			await http.request(
				"PUT",
				`${appPath(appId)}/db/${segment(table)}/update`,
				{ body: { filter, updates }, query: { ...options } },
			);
		},
		async addTableColumn(
			appId: string,
			table: string,
			body: {
				name: string;
				sql_expression?: string;
				data_type?: string;
				vector_size?: number;
			},
			options: DatabaseOptions = {},
		): Promise<void> {
			await http.request(
				"POST",
				`${appPath(appId)}/db/${segment(table)}/columns`,
				{ body, query: { ...options } },
			);
		},
		async alterTableColumn(
			appId: string,
			table: string,
			body: { column: string; rename?: string; nullable?: boolean },
			options: DatabaseOptions = {},
		): Promise<void> {
			await http.request(
				"PUT",
				`${appPath(appId)}/db/${segment(table)}/columns`,
				{ body, query: { ...options } },
			);
		},
		async dropTableColumns(
			appId: string,
			table: string,
			columns: string[],
			options: DatabaseOptions = {},
		): Promise<void> {
			await http.request(
				"DELETE",
				`${appPath(appId)}/db/${segment(table)}/columns`,
				{ body: { columns }, query: { ...options } },
			);
		},
		async setTablePrimaryKey(
			appId: string,
			table: string,
			column: string,
			options: DatabaseOptions = {},
		): Promise<void> {
			await http.request(
				"PUT",
				`${appPath(appId)}/db/${segment(table)}/primary-key`,
				{ body: { column }, query: { ...options } },
			);
		},
		async buildTableIndex(
			appId: string,
			table: string,
			body: { column: string; index_type: string; optimize: boolean },
			options: DatabaseOptions = {},
		): Promise<void> {
			await http.request(
				"POST",
				`${appPath(appId)}/db/${segment(table)}/index`,
				{ body, query: { ...options } },
			);
		},
		async dropTableIndex(
			appId: string,
			table: string,
			indexName: string,
			options: DatabaseOptions = {},
		): Promise<void> {
			await http.request(
				"DELETE",
				`${appPath(appId)}/db/${segment(table)}/index/${segment(indexName)}`,
				{ query: { ...options } },
			);
		},
		getTableIndices(
			appId: string,
			table: string,
			options: DatabaseOptions = {},
		): Promise<JsonObject[]> {
			return http.request(
				"GET",
				`${appPath(appId)}/db/${segment(table)}/indices`,
				{ query: { ...options } },
			);
		},
		async optimizeTable(
			appId: string,
			table: string,
			options: DatabaseOptions & { keep_versions?: boolean } = {},
		): Promise<void> {
			await http.request(
				"POST",
				`${appPath(appId)}/db/${segment(table)}/optimize`,
				{
					body: { keep_versions: options.keep_versions ?? true },
					query: {
						scope: options.scope,
						branch: options.branch,
						version: options.version,
						tag: options.tag,
						read_only: options.read_only,
					},
				},
			);
		},
		getTableView(
			appId: string,
			table: string,
			options: DatabaseOptions = {},
		): Promise<JsonObject> {
			return http.request(
				"GET",
				`${appPath(appId)}/db/${segment(table)}/view`,
				{ query: { ...options } },
			);
		},
		getTableHistory(
			appId: string,
			table: string,
			options: DatabaseOptions = {},
		): Promise<JsonObject> {
			return http.request(
				"GET",
				`${appPath(appId)}/db/${segment(table)}/references`,
				{ query: { ...options } },
			);
		},
		tableReferenceAction(
			appId: string,
			table: string,
			action: DatabaseAction,
			options: DatabaseOptions = {},
		): Promise<JsonObject> {
			return http.request(
				"POST",
				`${appPath(appId)}/db/${segment(table)}/references`,
				{ body: action, query: { ...options } },
			);
		},
		compareTableVersions(
			appId: string,
			table: string,
			body: {
				other: Omit<DatabaseOptions, "scope">;
				key: string;
				limit?: number;
			},
			options: DatabaseOptions = {},
		): Promise<JsonObject> {
			return http.request(
				"POST",
				`${appPath(appId)}/db/${segment(table)}/compare`,
				{ body, query: { ...options } },
			);
		},
		listSavedQueries(
			appId: string,
			options: DatabaseOptions = {},
		): Promise<JsonObject[]> {
			return http.request("GET", `${appPath(appId)}/db/queries`, {
				query: { scope: options.scope },
			});
		},
		getSavedQuery(
			appId: string,
			queryId: string,
			options: DatabaseOptions = {},
		): Promise<JsonObject> {
			return http.request(
				"GET",
				`${appPath(appId)}/db/queries/${segment(queryId)}`,
				{ query: { scope: options.scope } },
			);
		},
		createSavedQuery(
			appId: string,
			body: JsonObject,
			options: DatabaseOptions = {},
		): Promise<JsonObject> {
			return http.request("POST", `${appPath(appId)}/db/queries`, {
				body,
				query: { scope: options.scope },
			});
		},
		updateSavedQuery(
			appId: string,
			queryId: string,
			body: JsonObject,
			options: DatabaseOptions = {},
		): Promise<JsonObject> {
			return http.request(
				"PUT",
				`${appPath(appId)}/db/queries/${segment(queryId)}`,
				{ body, query: { scope: options.scope } },
			);
		},
		async deleteSavedQuery(
			appId: string,
			queryId: string,
			options: DatabaseOptions = {},
		): Promise<void> {
			await http.request(
				"DELETE",
				`${appPath(appId)}/db/queries/${segment(queryId)}`,
				{ query: { scope: options.scope } },
			);
		},
		executeQuery(
			appId: string,
			body: {
				sql: string;
				params?: unknown;
				surface?: string;
				overlay_id?: string;
				limit?: number;
			},
			options: DatabaseOptions = {},
		): Promise<JsonObject> {
			return http.request("POST", `${appPath(appId)}/db/queries/execute`, {
				body,
				query: { scope: options.scope },
			});
		},
		createTableBranch(
			appId: string,
			table: string,
			name: string,
			options: DatabaseOptions = {},
		): Promise<JsonObject> {
			return http.request(
				"POST",
				`${appPath(appId)}/db/${segment(table)}/references`,
				{ body: { action: "create_branch", name }, query: { ...options } },
			);
		},
		deleteTableBranch(
			appId: string,
			table: string,
			name: string,
			options: DatabaseOptions = {},
		): Promise<JsonObject> {
			return http.request(
				"POST",
				`${appPath(appId)}/db/${segment(table)}/references`,
				{ body: { action: "delete_branch", name }, query: { ...options } },
			);
		},
		createTableTag(
			appId: string,
			table: string,
			name: string,
			options: DatabaseOptions = {},
		): Promise<JsonObject> {
			return http.request(
				"POST",
				`${appPath(appId)}/db/${segment(table)}/references`,
				{ body: { action: "create_tag", name }, query: { ...options } },
			);
		},
		updateTableTag(
			appId: string,
			table: string,
			name: string,
			options: DatabaseOptions = {},
		): Promise<JsonObject> {
			return http.request(
				"POST",
				`${appPath(appId)}/db/${segment(table)}/references`,
				{ body: { action: "update_tag", name }, query: { ...options } },
			);
		},
		deleteTableTag(
			appId: string,
			table: string,
			name: string,
			options: DatabaseOptions = {},
		): Promise<JsonObject> {
			return http.request(
				"POST",
				`${appPath(appId)}/db/${segment(table)}/references`,
				{ body: { action: "delete_tag", name }, query: { ...options } },
			);
		},
		cloneTable(
			appId: string,
			table: string,
			name: string,
			options: DatabaseOptions = {},
		): Promise<JsonObject> {
			return http.request(
				"POST",
				`${appPath(appId)}/db/${segment(table)}/references`,
				{ body: { action: "clone", name }, query: { ...options } },
			);
		},
		restoreTable(
			appId: string,
			table: string,
			options: DatabaseOptions = {},
		): Promise<JsonObject> {
			return http.request(
				"POST",
				`${appPath(appId)}/db/${segment(table)}/references`,
				{ body: { action: "restore" }, query: { ...options } },
			);
		},
		snapshotTable(
			appId: string,
			table: string,
			name?: string,
			options: DatabaseOptions = {},
		): Promise<JsonObject> {
			return http.request(
				"POST",
				`${appPath(appId)}/db/${segment(table)}/references`,
				{ body: { action: "snapshot", name }, query: { ...options } },
			);
		},
		cleanupTableHistory(
			appId: string,
			table: string,
			olderThanDays: number,
			options: DatabaseOptions = {},
		): Promise<JsonObject> {
			return http.request(
				"POST",
				`${appPath(appId)}/db/${segment(table)}/references`,
				{
					body: { action: "cleanup", older_than_days: olderThanDays },
					query: { ...options },
				},
			);
		},
	};
}
