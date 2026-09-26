import type {
	IAddColumnPayload,
	ICreateTableResult,
	IDatabaseAction,
	IDatabaseActionResult,
	IDatabaseDiff,
	IDatabaseHistory,
	IDatabaseSchemaField,
	IDatabaseSelector,
	IDatabaseState,
	IDropTableResult,
	IIndexConfig,
	IIndexType,
	IQueryTablePayload,
	ITableSummary,
} from "../db-state";

export class EmptyDatabaseState implements IDatabaseState {
	databaseHistory(
		appId: string,
		tableName: string,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<IDatabaseHistory> {
		throw new Error("Database history is not available on this backend.");
	}
	databaseAction(
		appId: string,
		tableName: string,
		action: IDatabaseAction,
		userScoped?: boolean,
		selector?: IDatabaseSelector,
	): Promise<IDatabaseActionResult> {
		throw new Error("Database references are not available on this backend.");
	}
	createTable(
		appId: string,
		tableName: string,
		fields: IDatabaseSchemaField[],
		ifNotExists?: boolean,
	): Promise<ICreateTableResult> {
		throw new Error("Method not implemented.");
	}
	buildIndex(
		appId: string,
		tableName: string,
		column: string,
		indexType: IIndexType,
		optimize?: boolean,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	addItems(appId: string, tableName: string, items: any[]): Promise<void> {
		throw new Error("Method not implemented.");
	}
	removeItems(appId: string, tableName: string, query: string): Promise<void> {
		throw new Error("Method not implemented.");
	}
	listItems(
		appId: string,
		tableName: string,
		offset?: number,
		limit?: number,
	): Promise<any[]> {
		throw new Error("Method not implemented.");
	}
	queryItems(
		appId: string,
		tableName: string,
		query: IQueryTablePayload,
		offset?: number,
		limit?: number,
	): Promise<any[]> {
		throw new Error("Method not implemented.");
	}
	getSchema(appId: string, tableName: string): Promise<any> {
		throw new Error("Method not implemented.");
	}
	getSchemaAuthoritative(appId: string, tableName: string): Promise<any> {
		throw new Error("Method not implemented.");
	}
	getIndices(appId: string, tableName: string): Promise<IIndexConfig[]> {
		throw new Error("Method not implemented.");
	}
	dropIndex(
		appId: string,
		tableName: string,
		indexName: string,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	listTables(appId: string): Promise<string[]> {
		throw new Error("Method not implemented.");
	}
	listTablesAuthoritative(appId: string): Promise<string[]> {
		throw new Error("Method not implemented.");
	}
	listTablesUser(appId: string): Promise<string[]> {
		throw new Error("Method not implemented.");
	}
	listTableSummaries(
		appId: string,
		userScoped?: boolean,
	): Promise<ITableSummary[]> {
		throw new Error("Method not implemented.");
	}
	countItems(appId: string, tableName: string): Promise<number> {
		return Promise.resolve(0);
	}
	optimize(
		appId: string,
		tableName: string,
		keepVersions?: boolean,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	updateItem(
		appId: string,
		tableName: string,
		filter: string,
		updates: Record<string, any>,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	dropColumns(
		appId: string,
		tableName: string,
		columns: string[],
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	addColumn(
		appId: string,
		tableName: string,
		column: IAddColumnPayload,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	alterColumn(
		appId: string,
		tableName: string,
		column: string,
		nullable: boolean,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	setPrimaryKey(
		appId: string,
		tableName: string,
		column: string,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	dropTable(appId: string, tableName: string): Promise<IDropTableResult> {
		throw new Error("Method not implemented.");
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
		throw new Error("Database comparison is not available on this backend.");
	}
}
