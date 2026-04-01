import type { Connection as LanceConnection } from "@lancedb/lancedb";
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
	const creds = resp.shared_credentials;

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
		if (az.content_sas_token)
			opts.azure_storage_sas_token = az.content_sas_token;
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
		): Promise<LanceConnectionInfo> {
			const resp = await http.request<PresignDbAccessResponse>(
				"POST",
				`/apps/${appId}/db/presign`,
				{ body: { table_name: tableName, access_mode: accessMode } },
			);
			return resolveConnectionInfo(resp);
		},

		async getDbCredentialsRaw(
			appId: string,
			tableName = "_default",
			accessMode: "read" | "write" = "read",
		): Promise<PresignDbAccessResponse> {
			return http.request<PresignDbAccessResponse>(
				"POST",
				`/apps/${appId}/db/presign`,
				{ body: { table_name: tableName, access_mode: accessMode } },
			);
		},

		async createLanceConnection(
			appId: string,
			accessMode: "read" | "write" = "read",
		): Promise<LanceConnection> {
			let lancedb: typeof import("@lancedb/lancedb");
			try {
				lancedb = await import("@lancedb/lancedb");
			} catch {
				throw new Error(
					"@lancedb/lancedb is required for createLanceConnection. Install it with: npm install @lancedb/lancedb",
				);
			}

			const info = await this.getDbCredentials(appId, "_default", accessMode);
			return lancedb.connect(info.uri, {
				storageOptions: info.storageOptions,
			});
		},

		async listTables(appId: string): Promise<string[]> {
			return http.request<string[]>("GET", `/apps/${appId}/db/tables`);
		},

		async getTableSchema(appId: string, table: string): Promise<TableSchema> {
			return http.request<TableSchema>(
				"GET",
				`/apps/${appId}/db/${table}/schema`,
			);
		},

		async queryTable(
			appId: string,
			table: string,
			query: QueryOptions,
		): Promise<unknown[]> {
			return http.request<unknown[]>(
				"POST",
				`/apps/${appId}/db/${table}/query`,
				{ body: query },
			);
		},

		async addToTable(
			appId: string,
			table: string,
			data: unknown[],
		): Promise<void> {
			await http.request("POST", `/apps/${appId}/db/${table}/add`, {
				body: data,
			});
		},

		async deleteFromTable(
			appId: string,
			table: string,
			filter: string,
		): Promise<void> {
			await http.request("DELETE", `/apps/${appId}/db/${table}/delete`, {
				body: { filter },
			});
		},

		async countItems(appId: string, table: string): Promise<CountResult> {
			return http.request<CountResult>(
				"GET",
				`/apps/${appId}/db/${table}/count`,
			);
		},
	};
}
