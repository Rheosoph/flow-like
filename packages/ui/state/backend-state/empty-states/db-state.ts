import type {
	IAddColumnPayload,
	IDatabaseState,
	IIndexConfig,
	IIndexType,
	IQueryTablePayload,
} from "../db-state";

export class EmptyDatabaseState implements IDatabaseState {
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
	listTablesUser(appId: string): Promise<string[]> {
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
}
