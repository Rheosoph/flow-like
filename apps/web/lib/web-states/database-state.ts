import type { IDatabaseState } from "@flow-like/flow-like-ui";
import {
	type IDatabaseAction,
	type IDatabaseActionResult,
	type IDatabaseDiff,
	type IDatabaseHistory,
	type IDatabaseSelector,
	databaseQueryParams,
} from "@flow-like/flow-like-ui/state/backend-state/db-state";
import {
	type IAddColumnPayload,
	type ICreateTableResult,
	type IDatabaseSchemaField,
	type IDropTableResult,
	type IIndexConfig,
	type IIndexType,
	type IQueryTablePayload,
	type ITableSummary,
	indexTypeToString,
} from "@flow-like/flow-like-ui/state/backend-state/db-state";
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
	apiPost,
	apiPut,
} from "./api-utils";

export class WebDatabaseState implements IDatabaseState {
	constructor(private readonly backend: WebBackendRef) {}

	async createTable(
		appId: string,
		tableName: string,
		fields: IDatabaseSchemaField[],
		ifNotExists = true,
		userScoped?: boolean,
	): Promise<ICreateTableResult> {
		return apiPost<ICreateTableResult>(
			`apps/${appId}/db/${encodeURIComponent(tableName)}${this.scopeQuery(userScoped)}`,
			{ fields, if_not_exists: ifNotExists },
			this.backend.auth,
		);
	}

	private scopeQuery(
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): string {
		const params = databaseQueryParams(userScoped, selector).toString();
		return params ? `?${params}` : "";
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
		await apiPost(
			`apps/${appId}/db/${encodeURIComponent(tableName)}/index${this.scopeQuery(userScoped, selector)}`,
			{
				column,
				index_type: indexTypeToString(indexType),
				optimize: optimize ?? false,
			},
			this.backend.auth,
		);
	}

	async addItems(
		appId: string,
		tableName: string,
		items: any[],
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void> {
		await apiPut(
			`apps/${appId}/db/${encodeURIComponent(tableName)}${this.scopeQuery(userScoped, selector)}`,
			{ items },
			this.backend.auth,
		);
	}

	async removeItems(
		appId: string,
		tableName: string,
		query: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void> {
		await apiDelete(
			`apps/${appId}/db/${encodeURIComponent(tableName)}${this.scopeQuery(userScoped, selector)}`,
			this.backend.auth,
			{ query },
		);
	}

	private pageParams(
		offset?: number,
		limit?: number,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): URLSearchParams {
		const params = databaseQueryParams(userScoped, selector);
		if (offset !== undefined) params.set("offset", offset.toString());
		if (limit !== undefined) params.set("limit", limit.toString());
		return params;
	}

	async listItems(
		appId: string,
		tableName: string,
		offset?: number,
		limit?: number,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<any[]> {
		try {
			return await this.listItemsAuthoritative(
				appId,
				tableName,
				offset,
				limit,
				userScoped,
				selector,
			);
		} catch (error) {
			if (selector) throw error;
			return [];
		}
	}

	async listItemsAuthoritative(
		appId: string,
		tableName: string,
		offset?: number,
		limit?: number,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<unknown[]> {
		return apiGet<unknown[]>(
			`apps/${appId}/db/${encodeURIComponent(tableName)}?${this.pageParams(offset, limit, userScoped, selector)}`,
			this.backend.auth,
		);
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
		try {
			return await this.queryItemsAuthoritative(
				appId,
				tableName,
				query,
				offset,
				limit,
				userScoped,
				selector,
			);
		} catch (error) {
			if (selector) throw error;
			return [];
		}
	}

	async queryItemsAuthoritative(
		appId: string,
		tableName: string,
		query: IQueryTablePayload,
		offset?: number,
		limit?: number,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<unknown[]> {
		return apiPost<unknown[]>(
			`apps/${appId}/db/${encodeURIComponent(tableName)}/query?${this.pageParams(offset, limit, userScoped, selector)}`,
			{ ...query },
			this.backend.auth,
		);
	}

	async countItems(
		appId: string,
		tableName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<number> {
		try {
			return await this.countItemsAuthoritative(
				appId,
				tableName,
				userScoped,
				selector,
			);
		} catch (error) {
			if (selector) throw error;
			return 0;
		}
	}

	async countItemsAuthoritative(
		appId: string,
		tableName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<number> {
		const result = await apiGet<number>(
			`apps/${appId}/db/${encodeURIComponent(tableName)}/count${this.scopeQuery(userScoped, selector)}`,
			this.backend.auth,
		);
		return result ?? 0;
	}

	async getSchema(
		appId: string,
		tableName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<any> {
		return apiGet<any>(
			`apps/${appId}/db/${encodeURIComponent(tableName)}/schema${this.scopeQuery(userScoped, selector)}`,
			this.backend.auth,
		);
	}

	async getSchemaAuthoritative(
		appId: string,
		tableName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<any> {
		return apiGet<any>(
			`apps/${appId}/db/${encodeURIComponent(tableName)}/schema${this.scopeQuery(userScoped, selector)}`,
			this.backend.auth,
		);
	}

	async getIndices(
		appId: string,
		tableName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<IIndexConfig[]> {
		try {
			return await apiGet<IIndexConfig[]>(
				`apps/${appId}/db/${encodeURIComponent(tableName)}/indices${this.scopeQuery(userScoped, selector)}`,
				this.backend.auth,
			);
		} catch (error) {
			if (selector) throw error;
			return [];
		}
	}

	async dropIndex(
		appId: string,
		tableName: string,
		indexName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void> {
		await apiDelete(
			`apps/${appId}/db/${encodeURIComponent(tableName)}/index/${encodeURIComponent(indexName)}${this.scopeQuery(userScoped, selector)}`,
			this.backend.auth,
		);
	}

	async listTables(appId: string): Promise<string[]> {
		try {
			return await apiGet<string[]>(`apps/${appId}/db`, this.backend.auth);
		} catch {
			return [];
		}
	}

	async listTablesAuthoritative(appId: string): Promise<string[]> {
		return apiGet<string[]>(`apps/${appId}/db`, this.backend.auth);
	}

	async listTablesUser(appId: string): Promise<string[]> {
		try {
			return await apiGet<string[]>(`apps/${appId}/db/user`, this.backend.auth);
		} catch {
			return [];
		}
	}

	async listTableSummaries(
		appId: string,
		userScoped?: boolean,
	): Promise<ITableSummary[]> {
		try {
			return await apiGet<ITableSummary[]>(
				`apps/${appId}/db${userScoped ? "/user" : ""}?detail=summary`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async optimize(
		appId: string,
		tableName: string,
		keepVersions?: boolean,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void> {
		await apiPost(
			`apps/${appId}/db/${encodeURIComponent(tableName)}/optimize${this.scopeQuery(userScoped, selector)}`,
			{ keep_versions: keepVersions ?? true },
			this.backend.auth,
		);
	}

	async updateItem(
		appId: string,
		tableName: string,
		filter: string,
		updates: Record<string, any>,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void> {
		await apiPut(
			`apps/${appId}/db/${encodeURIComponent(tableName)}/update${this.scopeQuery(userScoped, selector)}`,
			{ filter, updates },
			this.backend.auth,
		);
	}

	async dropColumns(
		appId: string,
		tableName: string,
		columns: string[],
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void> {
		await apiDelete(
			`apps/${appId}/db/${encodeURIComponent(tableName)}/columns${this.scopeQuery(userScoped, selector)}`,
			this.backend.auth,
			{ columns },
		);
	}

	async addColumn(
		appId: string,
		tableName: string,
		column: IAddColumnPayload,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void> {
		await apiPost(
			`apps/${appId}/db/${encodeURIComponent(tableName)}/columns${this.scopeQuery(userScoped, selector)}`,
			column,
			this.backend.auth,
		);
	}

	async alterColumn(
		appId: string,
		tableName: string,
		column: string,
		nullable: boolean,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void> {
		await apiPut(
			`apps/${appId}/db/${encodeURIComponent(tableName)}/columns${this.scopeQuery(userScoped, selector)}`,
			{ column, nullable },
			this.backend.auth,
		);
	}

	async setPrimaryKey(
		appId: string,
		tableName: string,
		column: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<void> {
		await apiPut(
			`apps/${appId}/db/${encodeURIComponent(tableName)}/primary-key${this.scopeQuery(userScoped, selector)}`,
			{ column },
			this.backend.auth,
		);
	}

	async dropTable(
		appId: string,
		tableName: string,
		userScoped?: boolean,
	): Promise<IDropTableResult> {
		return apiDelete<IDropTableResult>(
			`apps/${appId}/db/${encodeURIComponent(tableName)}/table${this.scopeQuery(userScoped)}`,
			this.backend.auth,
		);
	}
	async databaseHistory(
		appId: string,
		tableName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<IDatabaseHistory> {
		return apiGet(
			`apps/${appId}/db/${encodeURIComponent(tableName)}/references${this.scopeQuery(userScoped, selector)}`,
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
		return apiPost(
			`apps/${appId}/db/${encodeURIComponent(tableName)}/references${this.scopeQuery(userScoped, selector)}`,
			action,
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
		return apiPost(
			`apps/${appId}/db/${encodeURIComponent(tableName)}/compare${this.scopeQuery(userScoped, selector)}`,
			{ other: otherSelector, key, limit },
			this.backend.auth,
		);
	}
}
