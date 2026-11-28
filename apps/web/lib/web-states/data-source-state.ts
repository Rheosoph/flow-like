import type {
	ExternalDataSource,
	ExternalDataSourceCheck,
	IDataSourceState,
	UpsertExternalDataSourcePayload,
} from "@flow-like/flow-like-ui";
import { asArray } from "@flow-like/flow-like-ui/lib/response-shape";
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
	apiPatch,
	apiPost,
} from "./api-utils";

function sourcesPath(appId: string): string {
	return `apps/${encodeURIComponent(appId)}/data-sources`;
}

function sourcePath(appId: string, sourceId: string): string {
	return `${sourcesPath(appId)}/${encodeURIComponent(sourceId)}`;
}

export class WebDataSourceState implements IDataSourceState {
	constructor(private readonly backend: WebBackendRef) {}

	async listDataSources(appId: string): Promise<ExternalDataSource[]> {
		return asArray(
			await apiGet<ExternalDataSource[]>(sourcesPath(appId), this.backend.auth),
		);
	}

	getDataSource(appId: string, sourceId: string): Promise<ExternalDataSource> {
		return apiGet<ExternalDataSource>(
			sourcePath(appId, sourceId),
			this.backend.auth,
		);
	}

	createDataSource(
		appId: string,
		payload: UpsertExternalDataSourcePayload,
	): Promise<ExternalDataSource> {
		return apiPost<ExternalDataSource>(
			sourcesPath(appId),
			payload,
			this.backend.auth,
		);
	}

	updateDataSource(
		appId: string,
		sourceId: string,
		payload: Partial<UpsertExternalDataSourcePayload>,
	): Promise<ExternalDataSource> {
		return apiPatch<ExternalDataSource>(
			sourcePath(appId, sourceId),
			payload,
			this.backend.auth,
		);
	}

	async deleteDataSource(appId: string, sourceId: string): Promise<void> {
		await apiDelete(sourcePath(appId, sourceId), this.backend.auth);
	}

	checkDataSource(
		appId: string,
		sourceId: string,
	): Promise<ExternalDataSourceCheck> {
		return apiPost<ExternalDataSourceCheck>(
			`${sourcePath(appId, sourceId)}/check`,
			undefined,
			this.backend.auth,
		);
	}
}
