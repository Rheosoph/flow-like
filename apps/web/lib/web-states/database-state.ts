import type { IDatabaseState } from "@tm9657/flow-like-ui";
import {
	type IAddColumnPayload,
	type IIndexConfig,
	IIndexType,
	type IQueryTablePayload,
} from "@tm9657/flow-like-ui/state/backend-state/db-state";
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
	apiPost,
	apiPut,
} from "./api-utils";

export class WebDatabaseState implements IDatabaseState {
	constructor(private readonly backend: WebBackendRef) {}

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

	private scopeParam(userScoped?: boolean): string {
		return userScoped ? "scope=user" : "";
	}

	private scopeQuery(userScoped?: boolean): string {
		return userScoped ? "?scope=user" : "";
	}

	async buildIndex(
		appId: string,
		tableName: string,
		column: string,
		indexType: IIndexType,
		optimize?: boolean,
		userScoped?: boolean,
	): Promise<void> {
		await apiPost(
			`apps/${appId}/db/${tableName}/index${this.scopeQuery(userScoped)}`,
			{
				column,
				index_type: this.indexTypeToString(indexType),
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
	): Promise<void> {
		await apiPost(
			`apps/${appId}/db/${tableName}/items${this.scopeQuery(userScoped)}`,
			{ items },
			this.backend.auth,
		);
	}

	async removeItems(
		appId: string,
		tableName: string,
		query: string,
		userScoped?: boolean,
	): Promise<void> {
		await apiPost(
			`apps/${appId}/db/${tableName}/delete${this.scopeQuery(userScoped)}`,
			{ query },
			this.backend.auth,
		);
	}

	async listItems(
		appId: string,
		tableName: string,
		offset?: number,
		limit?: number,
		userScoped?: boolean,
	): Promise<any[]> {
		const params = new URLSearchParams();
		if (offset !== undefined) params.set("offset", offset.toString());
		if (limit !== undefined) params.set("limit", limit.toString());
		if (userScoped) params.set("scope", "user");

		try {
			return await apiGet<any[]>(
				`apps/${appId}/db/${tableName}?${params}`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async queryItems(
		appId: string,
		tableName: string,
		query: IQueryTablePayload,
		offset?: number,
		limit?: number,
		userScoped?: boolean,
	): Promise<any[]> {
		const params = new URLSearchParams();
		if (offset !== undefined) params.set("offset", offset.toString());
		if (limit !== undefined) params.set("limit", limit.toString());
		if (userScoped) params.set("scope", "user");

		try {
			return await apiPost<any[]>(
				`apps/${appId}/db/${tableName}/query?${params}`,
				{ ...query },
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async countItems(appId: string, tableName: string, userScoped?: boolean): Promise<number> {
		try {
			const result = await apiGet<number>(
				`apps/${appId}/db/${tableName}/count${this.scopeQuery(userScoped)}`,
				this.backend.auth,
			);
			return result ?? 0;
		} catch {
			return 0;
		}
	}

	async getSchema(appId: string, tableName: string, userScoped?: boolean): Promise<any> {
		return apiGet<any>(
			`apps/${appId}/db/${tableName}/schema${this.scopeQuery(userScoped)}`,
			this.backend.auth,
		);
	}

	async getIndices(appId: string, tableName: string, userScoped?: boolean): Promise<IIndexConfig[]> {
		try {
			return await apiGet<IIndexConfig[]>(
				`apps/${appId}/db/${tableName}/indices${this.scopeQuery(userScoped)}`,
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async dropIndex(
		appId: string,
		tableName: string,
		indexName: string,
		userScoped?: boolean,
	): Promise<void> {
		await apiDelete(
			`apps/${appId}/db/${tableName}/index/${indexName}${this.scopeQuery(userScoped)}`,
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

	async listTablesUser(appId: string): Promise<string[]> {
		try {
			return await apiGet<string[]>(`apps/${appId}/db/user`, this.backend.auth);
		} catch {
			return [];
		}
	}

	async optimize(
		appId: string,
		tableName: string,
		keepVersions?: boolean,
		userScoped?: boolean,
	): Promise<void> {
		await apiPost(
			`apps/${appId}/db/${tableName}/optimize${this.scopeQuery(userScoped)}`,
			{ keep_versions: keepVersions },
			this.backend.auth,
		);
	}

	async updateItem(
		appId: string,
		tableName: string,
		filter: string,
		updates: Record<string, any>,
		userScoped?: boolean,
	): Promise<void> {
		await apiPut(
			`apps/${appId}/db/${tableName}/update${this.scopeQuery(userScoped)}`,
			{ filter, updates },
			this.backend.auth,
		);
	}

	async dropColumns(
		appId: string,
		tableName: string,
		columns: string[],
		userScoped?: boolean,
	): Promise<void> {
		await apiDelete(
			`apps/${appId}/db/${tableName}/columns${this.scopeQuery(userScoped)}`,
			this.backend.auth,
			{ columns },
		);
	}

	async addColumn(
		appId: string,
		tableName: string,
		column: IAddColumnPayload,
		userScoped?: boolean,
	): Promise<void> {
		await apiPost(
			`apps/${appId}/db/${tableName}/columns${this.scopeQuery(userScoped)}`,
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
	): Promise<void> {
		await apiPut(
			`apps/${appId}/db/${tableName}/columns${this.scopeQuery(userScoped)}`,
			{ column, nullable },
			this.backend.auth,
		);
	}
}
