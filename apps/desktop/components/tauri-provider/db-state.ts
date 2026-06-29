import { invoke } from "@tauri-apps/api/core";
import { IIndexType } from "@flow-like/flow-like-ui";
import type {
	IAddColumnPayload,
	IDatabaseState,
	IIndexConfig,
	IQueryTablePayload,
} from "@flow-like/flow-like-ui";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

function parseTableName(name: string): string {
	return encodeURIComponent(name);
}

function scopeQuery(userScoped?: boolean): string {
	return userScoped ? "scope=user" : "";
}

function appendScope(url: string, userScoped?: boolean): string {
	if (!userScoped) return url;
	return url.includes("?") ? `${url}&scope=user` : `${url}?scope=user`;
}

export class DatabaseState implements IDatabaseState {
	constructor(private readonly backend: TauriBackend) {}

	private indexTypeToString(indexType: IIndexType): string {
		const map: Record<IIndexType, string> = {
			[IIndexType.FullText]: "FullText",
			[IIndexType.BTree]: "BTree",
			[IIndexType.Bitmap]: "Bitmap",
			[IIndexType.LabelList]: "LabelList",
			[IIndexType.Auto]: "Auto",
		};
		return map[indexType] ?? "Auto";
	}

	async buildIndex(
		appId: string,
		tableName: string,
		column: string,
		indexType: IIndexType,
		optimize?: boolean,
		userScoped?: boolean,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/index`,
					userScoped,
				),
				{
					method: "POST",
					body: JSON.stringify({
						column,
						index_type: this.indexTypeToString(indexType),
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
			indexType,
			_optimize: optimize,
			userScoped: userScoped ?? false,
		});
	}

	async addItems(
		appId: string,
		tableName: string,
		items: any[],
		userScoped?: boolean,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}`,
					userScoped,
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
		});
	}

	async removeItems(
		appId: string,
		tableName: string,
		query: string,
		userScoped?: boolean,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}`,
					userScoped,
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
		});
	}

	async listItems(
		appId: string,
		tableName: string,
		offset?: number,
		limit?: number,
		userScoped?: boolean,
	): Promise<any[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}?offset=${offset ?? 0}&limit=${limit ?? 25}`,
					userScoped,
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
		});
	}

	async queryItems(
		appId: string,
		tableName: string,
		query: IQueryTablePayload,
		offset?: number,
		limit?: number,
		userScoped?: boolean,
	): Promise<any[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/query?offset=${offset ?? 0}&limit=${limit ?? 25}`,
					userScoped,
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
		});
	}

	async getSchema(
		appId: string,
		tableName: string,
		userScoped?: boolean,
	): Promise<any> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/schema`,
					userScoped,
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
		});
	}

	async getIndices(
		appId: string,
		tableName: string,
		userScoped?: boolean,
	): Promise<IIndexConfig[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/indices`,
					userScoped,
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
		});
	}

	async dropIndex(
		appId: string,
		tableName: string,
		indexName: string,
		userScoped?: boolean,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/index/${encodeURIComponent(indexName)}`,
					userScoped,
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

	async countItems(
		appId: string,
		tableName: string,
		userScoped?: boolean,
	): Promise<number> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/count`,
					userScoped,
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
		});
	}

	async optimize(
		appId: string,
		tableName: string,
		keepVersions?: boolean,
		userScoped?: boolean,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/optimize`,
					userScoped,
				),
				{
					method: "POST",
					body: JSON.stringify({ keep_versions: keepVersions ?? false }),
				},
				this.backend.auth,
			);
		}

		return await invoke("db_optimize", {
			appId,
			tableName,
			keepVersions: keepVersions ?? false,
			userScoped: userScoped ?? false,
		});
	}

	async updateItem(
		appId: string,
		tableName: string,
		filter: string,
		updates: Record<string, any>,
		userScoped?: boolean,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/update`,
					userScoped,
				),
				{
					method: "POST",
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
		});
	}

	async dropColumns(
		appId: string,
		tableName: string,
		columns: string[],
		userScoped?: boolean,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/columns`,
					userScoped,
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
		});
	}

	async addColumn(
		appId: string,
		tableName: string,
		column: IAddColumnPayload,
		userScoped?: boolean,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/columns`,
					userScoped,
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
		});
	}

	async alterColumn(
		appId: string,
		tableName: string,
		column: string,
		nullable: boolean,
		userScoped?: boolean,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/${parseTableName(tableName)}/columns`,
					userScoped,
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
		});
	}
}
