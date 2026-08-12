export enum IIndexType {
	FullText = 0,
	BTree = 1,
	Bitmap = 2,
	LabelList = 3,
	Auto = 4,
}

export interface IQueryTableVectorPayload {
	column: string;
	vector: number[];
}

export interface IQueryTablePayload {
	sql?: string;
	vector_query?: IQueryTableVectorPayload;
	filter?: string;
	fts_term?: string;
	rerank?: boolean;
}

export interface IIndexConfig {
	name: string;
	index_type: string;
	columns: string[];
}

export interface IAddColumnPayload {
	name: string;
	sql_expression: string;
}

export interface IDatabaseSchemaField {
	name: string;
	type: string;
	nullable?: boolean;
	vector_size?: number;
}

export interface ICreateTableResult {
	table_name: string;
	created: boolean;
	if_not_exists: boolean;
}

export interface IDropTableResult {
	table_name: string;
	dropped: boolean;
	ontologies: string[];
	saved_queries: string[];
	warnings: string[];
}

export interface IDatabaseState {
	createTable(
		appId: string,
		tableName: string,
		fields: IDatabaseSchemaField[],
		ifNotExists?: boolean,
		userScoped?: boolean,
	): Promise<ICreateTableResult>;
	buildIndex(
		appId: string,
		tableName: string,
		column: string,
		indexType: IIndexType,
		optimize?: boolean,
		userScoped?: boolean,
	): Promise<void>;
	addItems(
		appId: string,
		tableName: string,
		items: any[],
		userScoped?: boolean,
	): Promise<void>;
	removeItems(
		appId: string,
		tableName: string,
		query: string,
		userScoped?: boolean,
	): Promise<void>;
	listItems(
		appId: string,
		tableName: string,
		offset?: number,
		limit?: number,
		userScoped?: boolean,
	): Promise<any[]>;
	queryItems(
		appId: string,
		tableName: string,
		query: IQueryTablePayload,
		offset?: number,
		limit?: number,
		userScoped?: boolean,
	): Promise<any[]>;
	countItems(
		appId: string,
		tableName: string,
		userScoped?: boolean,
	): Promise<number>;
	getSchema(
		appId: string,
		tableName: string,
		userScoped?: boolean,
	): Promise<any>;
	getIndices(
		appId: string,
		tableName: string,
		userScoped?: boolean,
	): Promise<IIndexConfig[]>;
	dropIndex(
		appId: string,
		tableName: string,
		indexName: string,
		userScoped?: boolean,
	): Promise<void>;
	listTables(appId: string): Promise<string[]>;
	listTablesUser(appId: string): Promise<string[]>;
	optimize(
		appId: string,
		tableName: string,
		keepVersions?: boolean,
		userScoped?: boolean,
	): Promise<void>;
	updateItem(
		appId: string,
		tableName: string,
		filter: string,
		updates: Record<string, any>,
		userScoped?: boolean,
	): Promise<void>;
	dropColumns(
		appId: string,
		tableName: string,
		columns: string[],
		userScoped?: boolean,
	): Promise<void>;
	addColumn(
		appId: string,
		tableName: string,
		column: IAddColumnPayload,
		userScoped?: boolean,
	): Promise<void>;
	alterColumn(
		appId: string,
		tableName: string,
		column: string,
		nullable: boolean,
		userScoped?: boolean,
	): Promise<void>;
	dropTable(
		appId: string,
		tableName: string,
		userScoped?: boolean,
	): Promise<IDropTableResult>;
}
