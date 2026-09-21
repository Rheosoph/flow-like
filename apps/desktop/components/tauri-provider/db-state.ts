import type {
	IAddColumnPayload,
	ICreateTableResult,
	IDatabaseSchemaField,
	IDatabaseState,
	IDropTableResult,
	IIndexConfig,
	IIndexType,
	IQueryTablePayload,
	ITableSummary,
} from "@flow-like/flow-like-ui";
import {
	type IDatabaseAction,
	type IDatabaseActionResult,
	type IDatabaseDiff,
	type IDatabaseHistory,
	type IDatabaseSelector,
	databaseQueryParams,
} from "@flow-like/flow-like-ui/state/backend-state/db-state";
import { indexTypeToString } from "@flow-like/flow-like-ui/state/backend-state/db-state";
import { invoke } from "@tauri-apps/api/core";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

function parseTableName(name: string): string {
	return encodeURIComponent(name);
}

function appendScope(
	url: string,
	userScoped?: boolean,
	selector?: IDatabaseSelector,
): string {
	const params = databaseQueryParams(userScoped, selector).toString();
	if (!params) return url;
	return `${url}${url.includes("?") ? "&" : "?"}${params}`;
}

export class DatabaseState implements IDatabaseState {
	constructor(private readonly backend: TauriBackend) {}

	async createTable(
		appId: string,
		tableName: string,
		fields: IDatabaseSchemaField[],
		ifNotExists = true,
		userScoped?: boolean,
	): Promise<ICreateTableResult> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}`,
					userScoped,
				),
				{
					method: "POST",
					body: JSON.stringify({ fields, if_not_exists: ifNotExists }),
				},
				this.backend.auth,
			);
		}

		return await invoke<ICreateTableResult>("db_create_table", {
			appId,
			tableName,
			fields,
			ifNotExists,
			userScoped: userScoped ?? false,
		});
	}

	async buildIndex(
		appId: string,
		tableName: string,
		column: string,
		indexType: IIndexType,
		optimize?: boolean,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/index`,
					userScoped,
					selector,
				),
				{
					method: "POST",
					body: JSON.stringify({
						column,
						index_type: indexTypeToString(indexType),
						optimize: optimize ?? false,
					}),
				},
				this.backend.auth,
			);
		}

		return await invoke("build_index", {
			appId,
			tableName,
			column,
			indexType: indexTypeToString(indexType),
			optimize,
			userScoped: userScoped ?? false,
			...(selector ? { selector } : {}),
		});
	}

	async addItems(
		appId: string,
		tableName: string,
		items: any[],
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}`,
					userScoped,
					selector,
				),
				{
					method: "PUT",
					body: JSON.stringify({
						items,
					}),
				},
				this.backend.auth,
			);
		}

		return await invoke("db_add", {
			appId,
			tableName,
			items,
			userScoped: userScoped ?? false,
			...(selector ? { selector } : {}),
		});
	}

	async removeItems(
		appId: string,
		tableName: string,
		query: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}`,
					userScoped,
					selector,
				),
				{
					method: "DELETE",
					body: JSON.stringify({
						query,
					}),
				},
				this.backend.auth,
			);
		}

		return await invoke("db_delete", {
			appId,
			tableName,
			query,
			userScoped: userScoped ?? false,
			...(selector ? { selector } : {}),
		});
	}

	async listItems(
		appId: string,
		tableName: string,
		offset?: number,
		limit?: number,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<any[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}?offset=${offset ?? 0}&limit=${limit ?? 25}`,
					userScoped,
					selector,
				),
				{
					method: "GET",
				},
				this.backend.auth,
			);
		}

		return await invoke("db_list", {
			appId,
			tableName,
			offset,
			limit,
			userScoped: userScoped ?? false,
			...(selector ? { selector } : {}),
		});
	}

	async queryItems(
		appId: string,
		tableName: string,
		query: IQueryTablePayload,
		offset?: number,
		limit?: number,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<any[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/query?offset=${offset ?? 0}&limit=${limit ?? 25}`,
					userScoped,
					selector,
				),
				{
					method: "POST",
					body: JSON.stringify(query),
				},
				this.backend.auth,
			);
		}

		return await invoke("db_query", {
			appId,
			tableName,
			payload: query,
			offset,
			limit,
			userScoped: userScoped ?? false,
			...(selector ? { selector } : {}),
		});
	}

	async getSchema(
		appId: string,
		tableName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<any> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/schema`,
					userScoped,
					selector,
				),
				{
					method: "GET",
				},
				this.backend.auth,
			);
		}

		return await invoke<any>("db_schema", {
			appId,
			tableName,
			userScoped: userScoped ?? false,
			...(selector ? { selector } : {}),
		});
	}

	async getSchemaAuthoritative(
		appId: string,
		tableName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<any> {
		if (await this.backend.isLocalOnly(appId)) {
			return invoke<any>("db_schema", {
				appId,
				tableName,
				userScoped: userScoped ?? false,
				...(selector ? { selector } : {}),
			});
		}
		if (
			!this.backend.profile ||
			!this.backend.auth?.isAuthenticated ||
			!this.backend.auth.user?.access_token
		) {
			throw new Error(
				"Hosted database schema reads require an authenticated hub session",
			);
		}
		return fetcher(
			this.backend.profile,
			appendScope(
				`apps/${appId}/db/${parseTableName(tableName)}/schema`,
				userScoped,
				selector,
			),
			{ method: "GET" },
			this.backend.auth,
		);
	}

	async getIndices(
		appId: string,
		tableName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<IIndexConfig[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/indices`,
					userScoped,
					selector,
				),
				{
					method: "GET",
				},
				this.backend.auth,
			);
		}

		return await invoke("db_indices", {
			appId,
			tableName,
			userScoped: userScoped ?? false,
			...(selector ? { selector } : {}),
		});
	}

	async dropIndex(
		appId: string,
		tableName: string,
		indexName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/index/${encodeURIComponent(indexName)}`,
					userScoped,
					selector,
				),
				{
					method: "DELETE",
				},
				this.backend.auth,
			);
			return;
		}

		await invoke("db_drop_index", {
			appId,
			tableName,
			indexName,
			userScoped: userScoped ?? false,
			...(selector ? { selector } : {}),
		});
	}

	async listTables(appId: string): Promise<string[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				`apps/${appId}/db`,
				{
					method: "GET",
				},
				this.backend.auth,
			);
		}

		return await invoke("db_table_names", { appId });
	}

	async listTablesAuthoritative(appId: string): Promise<string[]> {
		if (await this.backend.isLocalOnly(appId)) {
			return invoke<string[]>("db_table_names", { appId });
		}
		if (
			!this.backend.profile ||
			!this.backend.auth?.isAuthenticated ||
			!this.backend.auth.user?.access_token
		) {
			throw new Error(
				"Hosted database inventory requires an authenticated hub session",
			);
		}
		return fetcher<string[]>(
			this.backend.profile,
			`apps/${appId}/db`,
			{ method: "GET" },
			this.backend.auth,
		);
	}

	async listTablesUser(appId: string): Promise<string[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				`apps/${appId}/db/user`,
				{
					method: "GET",
				},
				this.backend.auth,
			);
		}

		return await invoke("db_table_names_user", { appId });
	}

	async listTableSummaries(
		appId: string,
		userScoped?: boolean,
	): Promise<ITableSummary[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				`apps/${appId}/db${userScoped ? "/user" : ""}?detail=summary`,
				{
					method: "GET",
				},
				this.backend.auth,
			);
		}

		return await invoke(
			userScoped ? "db_table_summaries_user" : "db_table_summaries",
			{ appId },
		);
	}

	async countItems(
		appId: string,
		tableName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<number> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/count`,
					userScoped,
					selector,
				),
				{
					method: "GET",
				},
				this.backend.auth,
			);
		}

		return await invoke("db_count", {
			appId,
			tableName,
			userScoped: userScoped ?? false,
			...(selector ? { selector } : {}),
		});
	}

	async optimize(
		appId: string,
		tableName: string,
		keepVersions?: boolean,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/optimize`,
					userScoped,
					selector,
				),
				{
					method: "POST",
					body: JSON.stringify({ keep_versions: keepVersions ?? true }),
				},
				this.backend.auth,
			);
		}

		return await invoke("db_optimize", {
			appId,
			tableName,
			keepVersions: keepVersions ?? true,
			userScoped: userScoped ?? false,
			...(selector ? { selector } : {}),
		});
	}

	async updateItem(
		appId: string,
		tableName: string,
		filter: string,
		updates: Record<string, any>,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/update`,
					userScoped,
					selector,
				),
				{
					method: "PUT",
					body: JSON.stringify({ filter, updates }),
				},
				this.backend.auth,
			);
		}

		return await invoke("db_update", {
			appId,
			tableName,
			filter,
			updates,
			userScoped: userScoped ?? false,
			...(selector ? { selector } : {}),
		});
	}

	async dropColumns(
		appId: string,
		tableName: string,
		columns: string[],
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/columns`,
					userScoped,
					selector,
				),
				{
					method: "DELETE",
					body: JSON.stringify({ columns }),
				},
				this.backend.auth,
			);
		}

		return await invoke("db_drop_columns", {
			appId,
			tableName,
			columns,
			userScoped: userScoped ?? false,
			...(selector ? { selector } : {}),
		});
	}

	async addColumn(
		appId: string,
		tableName: string,
		column: IAddColumnPayload,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/columns`,
					userScoped,
					selector,
				),
				{
					method: "POST",
					body: JSON.stringify(column),
				},
				this.backend.auth,
			);
		}

		return await invoke("db_add_column", {
			appId,
			tableName,
			column,
			userScoped: userScoped ?? false,
			...(selector ? { selector } : {}),
		});
	}

	async alterColumn(
		appId: string,
		tableName: string,
		column: string,
		nullable: boolean,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/columns`,
					userScoped,
					selector,
				),
				{
					method: "PUT",
					body: JSON.stringify({ column, nullable }),
				},
				this.backend.auth,
			);
		}

		return await invoke("db_alter_column", {
			appId,
			tableName,
			column,
			nullable,
			userScoped: userScoped ?? false,
			...(selector ? { selector } : {}),
		});
	}

	async dropTable(
		appId: string,
		tableName: string,
		userScoped?: boolean,
	): Promise<IDropTableResult> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/table`,
					userScoped,
				),
				{
					method: "DELETE",
				},
				this.backend.auth,
			);
		}

		return await invoke<IDropTableResult>("db_drop_table", {
			appId,
			tableName,
			userScoped: userScoped ?? false,
		});
	}
	async databaseHistory(
		appId: string,
		tableName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<IDatabaseHistory> {
		if (await this.backend.isOffline(appId)) {
			return invoke("db_history", {
				appId,
				tableName,
				userScoped: userScoped ?? false,
				selector,
			});
		}
		return fetcher(
			this.backend.profile!,
			appendScope(
				`apps/${appId}/db/${parseTableName(tableName)}/references`,
				userScoped,
				selector,
			),
			{ method: "GET" },
			this.backend.auth,
		);
	}

	async databaseAction(
		appId: string,
		tableName: string,
		action: IDatabaseAction,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<IDatabaseActionResult> {
		if (await this.backend.isOffline(appId)) {
			return invoke("db_reference_action", {
				appId,
				tableName,
				action,
				userScoped: userScoped ?? false,
				selector,
			});
		}
		return fetcher(
			this.backend.profile!,
			appendScope(
				`apps/${appId}/db/${parseTableName(tableName)}/references`,
				userScoped,
				selector,
			),
			{ method: "POST", body: JSON.stringify(action) },
			this.backend.auth,
		);
	}
	async databaseCompare(
		appId: string,
		tableName: string,
		otherSelector: IDatabaseSelector,
		key: string,
		limit?: number,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<IDatabaseDiff> {
		if (await this.backend.isOffline(appId)) {
			return invoke("db_compare", {
				appId,
				tableName,
				other: otherSelector,
				key,
				limit,
				userScoped: userScoped ?? false,
				selector,
			});
		}
		return fetcher(
			this.backend.profile!,
			appendScope(
				`apps/${appId}/db/${parseTableName(tableName)}/compare`,
				userScoped,
				selector,
			),
			{
				method: "POST",
				body: JSON.stringify({ other: otherSelector, key, limit }),
			},
			this.backend.auth,
		);
	}
}
