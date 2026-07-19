import type {
	CreateSavedQueryPayload,
	ExecuteSqlPayload,
	ExecuteSqlResult,
	IQueryState,
	SavedQuery,
	UpdateSavedQueryPayload,
} from "../query-state";

export class EmptyQueryState implements IQueryState {
	executeSql(
		appId: string,
		payload: ExecuteSqlPayload,
		userScoped?: boolean,
	): Promise<ExecuteSqlResult> {
		throw new Error("Method not implemented.");
	}
	listSavedQueries(appId: string, userScoped?: boolean): Promise<SavedQuery[]> {
		return Promise.resolve([]);
	}
	getSavedQuery(
		appId: string,
		queryId: string,
		userScoped?: boolean,
	): Promise<SavedQuery> {
		throw new Error("Method not implemented.");
	}
	createSavedQuery(
		appId: string,
		payload: CreateSavedQueryPayload,
		userScoped?: boolean,
	): Promise<SavedQuery> {
		throw new Error("Method not implemented.");
	}
	updateSavedQuery(
		appId: string,
		queryId: string,
		payload: UpdateSavedQueryPayload,
		userScoped?: boolean,
	): Promise<SavedQuery> {
		throw new Error("Method not implemented.");
	}
	deleteSavedQuery(
		appId: string,
		queryId: string,
		userScoped?: boolean,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
}
