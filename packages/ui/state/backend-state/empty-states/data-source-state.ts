import type {
	ExternalDataSource,
	ExternalDataSourceCheck,
	IDataSourceState,
	UpsertExternalDataSourcePayload,
} from "../data-source-state";

const UNAVAILABLE = "External databases are unavailable in this backend.";

export class EmptyDataSourceState implements IDataSourceState {
	listDataSources(_appId: string): Promise<ExternalDataSource[]> {
		return Promise.resolve([]);
	}
	getDataSource(
		_appId: string,
		_sourceId: string,
	): Promise<ExternalDataSource> {
		return Promise.reject(new Error(UNAVAILABLE));
	}
	createDataSource(
		_appId: string,
		_payload: UpsertExternalDataSourcePayload,
	): Promise<ExternalDataSource> {
		return Promise.reject(new Error(UNAVAILABLE));
	}
	updateDataSource(
		_appId: string,
		_sourceId: string,
		_payload: Partial<UpsertExternalDataSourcePayload>,
	): Promise<ExternalDataSource> {
		return Promise.reject(new Error(UNAVAILABLE));
	}
	deleteDataSource(_appId: string, _sourceId: string): Promise<void> {
		return Promise.reject(new Error(UNAVAILABLE));
	}
	checkDataSource(
		_appId: string,
		_sourceId: string,
	): Promise<ExternalDataSourceCheck> {
		return Promise.reject(new Error(UNAVAILABLE));
	}
}
