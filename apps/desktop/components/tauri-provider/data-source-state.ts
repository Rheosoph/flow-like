import type {
	ExternalDataSource,
	ExternalDataSourceCheck,
	IDataSourceState,
	UpsertExternalDataSourcePayload,
} from "@flow-like/flow-like-ui";
import { asArray } from "@flow-like/flow-like-ui/lib/response-shape";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

function sourcesPath(appId: string): string {
	return `apps/${encodeURIComponent(appId)}/data-sources`;
}

function sourcePath(appId: string, sourceId: string): string {
	return `${sourcesPath(appId)}/${encodeURIComponent(sourceId)}`;
}

/** External databases live on the hub: the query service reads them, never this device. */
export class DataSourceState implements IDataSourceState {
	constructor(private readonly backend: TauriBackend) {}

	private async request<T>(
		appId: string,
		path: string,
		init: RequestInit,
	): Promise<T> {
		if (await this.backend.isOffline(appId)) {
			throw new Error("External databases need an online app.");
		}
		const profile = this.backend.profile;
		if (!profile) {
			throw new Error("Select a profile before managing external databases.");
		}
		return fetcher<T>(profile, path, init, this.backend.auth);
	}

	async listDataSources(appId: string): Promise<ExternalDataSource[]> {
		return asArray(
			await this.request<ExternalDataSource[]>(appId, sourcesPath(appId), {
				method: "GET",
			}),
		);
	}

	getDataSource(appId: string, sourceId: string): Promise<ExternalDataSource> {
		return this.request(appId, sourcePath(appId, sourceId), { method: "GET" });
	}

	createDataSource(
		appId: string,
		payload: UpsertExternalDataSourcePayload,
	): Promise<ExternalDataSource> {
		return this.request(appId, sourcesPath(appId), {
			method: "POST",
			body: JSON.stringify(payload),
		});
	}

	updateDataSource(
		appId: string,
		sourceId: string,
		payload: Partial<UpsertExternalDataSourcePayload>,
	): Promise<ExternalDataSource> {
		return this.request(appId, sourcePath(appId, sourceId), {
			method: "PATCH",
			body: JSON.stringify(payload),
		});
	}

	async deleteDataSource(appId: string, sourceId: string): Promise<void> {
		await this.request(appId, sourcePath(appId, sourceId), {
			method: "DELETE",
		});
	}

	checkDataSource(
		appId: string,
		sourceId: string,
	): Promise<ExternalDataSourceCheck> {
		return this.request(appId, `${sourcePath(appId, sourceId)}/check`, {
			method: "POST",
		});
	}
}
