import type {
	CreateSavedQueryPayload,
	ExecuteSqlPayload,
	ExecuteSqlResult,
	IQueryState,
	SavedQuery,
	UpdateSavedQueryPayload,
} from "@flow-like/flow-like-ui";
import { invoke } from "@tauri-apps/api/core";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

function appendScope(url: string, userScoped?: boolean): string {
	if (!userScoped) return url;
	return url.includes("?") ? `${url}&scope=user` : `${url}?scope=user`;
}

export class QueryState implements IQueryState {
	constructor(private readonly backend: TauriBackend) {}

	async executeSql(
		appId: string,
		payload: ExecuteSqlPayload,
		userScoped?: boolean,
	): Promise<ExecuteSqlResult> {
		const isOffline = await this.backend.isOffline(appId);
		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(`apps/${appId}/db/queries/execute`, userScoped),
				{ method: "POST", body: JSON.stringify(payload) },
				this.backend.auth,
			);
		}
		return await invoke<ExecuteSqlResult>("query_execute_sql", {
			appId,
			payload,
			userScoped: userScoped ?? false,
		});
	}

	async listSavedQueries(
		appId: string,
		userScoped?: boolean,
	): Promise<SavedQuery[]> {
		const isOffline = await this.backend.isOffline(appId);
		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(`apps/${appId}/db/queries`, userScoped),
				{ method: "GET" },
				this.backend.auth,
			);
		}
		return await invoke<SavedQuery[]>("query_saved_list", {
			appId,
			userScoped: userScoped ?? false,
		});
	}

	async getSavedQuery(
		appId: string,
		queryId: string,
		userScoped?: boolean,
	): Promise<SavedQuery> {
		const isOffline = await this.backend.isOffline(appId);
		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/queries/${encodeURIComponent(queryId)}`,
					userScoped,
				),
				{ method: "GET" },
				this.backend.auth,
			);
		}
		return await invoke<SavedQuery>("query_saved_get", {
			appId,
			queryId,
			userScoped: userScoped ?? false,
		});
	}

	async createSavedQuery(
		appId: string,
		payload: CreateSavedQueryPayload,
		userScoped?: boolean,
	): Promise<SavedQuery> {
		const isOffline = await this.backend.isOffline(appId);
		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(`apps/${appId}/db/queries`, userScoped),
				{ method: "POST", body: JSON.stringify(payload) },
				this.backend.auth,
			);
		}
		return await invoke<SavedQuery>("query_saved_create", {
			appId,
			payload,
			userScoped: userScoped ?? false,
		});
	}

	async updateSavedQuery(
		appId: string,
		queryId: string,
		payload: UpdateSavedQueryPayload,
		userScoped?: boolean,
	): Promise<SavedQuery> {
		const isOffline = await this.backend.isOffline(appId);
		if (!isOffline) {
			return await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/queries/${encodeURIComponent(queryId)}`,
					userScoped,
				),
				{ method: "PUT", body: JSON.stringify(payload) },
				this.backend.auth,
			);
		}
		return await invoke<SavedQuery>("query_saved_update", {
			appId,
			queryId,
			payload,
			userScoped: userScoped ?? false,
		});
	}

	async deleteSavedQuery(
		appId: string,
		queryId: string,
		userScoped?: boolean,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);
		if (!isOffline) {
			await fetcher(
				this.backend.profile!,
				appendScope(
					`apps/${appId}/db/queries/${encodeURIComponent(queryId)}`,
					userScoped,
				),
				{ method: "DELETE" },
				this.backend.auth,
			);
			return;
		}
		await invoke("query_saved_delete", {
			appId,
			queryId,
			userScoped: userScoped ?? false,
		});
	}
}
