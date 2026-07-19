import type {
	CreateSavedQueryPayload,
	ExecuteSqlPayload,
	ExecuteSqlResult,
	IQueryState,
	SavedQuery,
	UpdateSavedQueryPayload,
} from "@flow-like/flow-like-ui";
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
	apiPost,
	apiPut,
} from "./api-utils";

function scopeQuery(userScoped?: boolean): string {
	return userScoped ? "?scope=user" : "";
}

export class WebQueryState implements IQueryState {
	constructor(private readonly backend: WebBackendRef) {}

	async executeSql(
		appId: string,
		payload: ExecuteSqlPayload,
		userScoped?: boolean,
	): Promise<ExecuteSqlResult> {
		return apiPost<ExecuteSqlResult>(
			`apps/${appId}/db/queries/execute${scopeQuery(userScoped)}`,
			payload,
			this.backend.auth,
		);
	}

	async listSavedQueries(
		appId: string,
		userScoped?: boolean,
	): Promise<SavedQuery[]> {
		return apiGet<SavedQuery[]>(
			`apps/${appId}/db/queries${scopeQuery(userScoped)}`,
			this.backend.auth,
		);
	}

	async getSavedQuery(
		appId: string,
		queryId: string,
		userScoped?: boolean,
	): Promise<SavedQuery> {
		return apiGet<SavedQuery>(
			`apps/${appId}/db/queries/${encodeURIComponent(queryId)}${scopeQuery(userScoped)}`,
			this.backend.auth,
		);
	}

	async createSavedQuery(
		appId: string,
		payload: CreateSavedQueryPayload,
		userScoped?: boolean,
	): Promise<SavedQuery> {
		return apiPost<SavedQuery>(
			`apps/${appId}/db/queries${scopeQuery(userScoped)}`,
			payload,
			this.backend.auth,
		);
	}

	async updateSavedQuery(
		appId: string,
		queryId: string,
		payload: UpdateSavedQueryPayload,
		userScoped?: boolean,
	): Promise<SavedQuery> {
		return apiPut<SavedQuery>(
			`apps/${appId}/db/queries/${encodeURIComponent(queryId)}${scopeQuery(userScoped)}`,
			payload,
			this.backend.auth,
		);
	}

	async deleteSavedQuery(
		appId: string,
		queryId: string,
		userScoped?: boolean,
	): Promise<void> {
		await apiDelete<void>(
			`apps/${appId}/db/queries/${encodeURIComponent(queryId)}${scopeQuery(userScoped)}`,
			this.backend.auth,
		);
	}
}
