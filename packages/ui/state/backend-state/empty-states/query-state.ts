import type {
	CreateSavedQueryPayload,
	CreateStudioForecastPayload,
	CrossAppSqlPayload,
	CrossAppSqlResult,
	ExecuteSqlPayload,
	ExecuteSqlResult,
	IQueryState,
	SavedQuery,
	SavedStudioForecast,
	SqlParameterOptions,
	SqlParameterOptionsRequest,
	SqlQueryDescription,
	StudioForecastPayload,
	StudioForecastResult,
	StudioForecastSummary,
	UpdateSavedQueryPayload,
} from "../query-state";

export class EmptyQueryState implements IQueryState {
	listForecasts(
		_appId: string,
		_userScoped?: boolean,
		_maximum?: number,
	): Promise<StudioForecastSummary[]> {
		return Promise.resolve([]);
	}
	getForecast(
		_appId: string,
		_forecastId: string,
		_userScoped?: boolean,
	): Promise<SavedStudioForecast> {
		return Promise.reject(
			new Error("Saved predictions are unavailable in this backend."),
		);
	}
	createForecast(
		_appId: string,
		_payload: CreateStudioForecastPayload,
		_userScoped?: boolean,
	): Promise<SavedStudioForecast> {
		return Promise.reject(
			new Error("Saved predictions are unavailable in this backend."),
		);
	}
	deleteForecast(
		_appId: string,
		_forecastId: string,
		_userScoped?: boolean,
	): Promise<void> {
		return Promise.reject(
			new Error("Saved predictions are unavailable in this backend."),
		);
	}
	forecast(
		_appId: string,
		_payload: StudioForecastPayload,
		_userScoped?: boolean,
	): Promise<StudioForecastResult> {
		return Promise.reject(
			new Error("Forecasting is unavailable in this backend."),
		);
	}
	executeSql(
		appId: string,
		payload: ExecuteSqlPayload,
		userScoped?: boolean,
	): Promise<ExecuteSqlResult> {
		throw new Error("Method not implemented.");
	}
	executeCrossAppSql(_payload: CrossAppSqlPayload): Promise<CrossAppSqlResult> {
		throw new Error("Method not implemented.");
	}
	describeCrossAppSql(
		_payload: Pick<CrossAppSqlPayload, "sql" | "sources">,
	): Promise<SqlQueryDescription> {
		throw new Error("Method not implemented.");
	}
	crossAppSqlParameterOptions(
		_payload: Pick<CrossAppSqlPayload, "sources"> & SqlParameterOptionsRequest,
	): Promise<SqlParameterOptions> {
		throw new Error("Method not implemented.");
	}
	describeSql(
		_appId: string,
		_payload: Pick<ExecuteSqlPayload, "sql" | "surface" | "overlay_id">,
		_userScoped?: boolean,
	): Promise<SqlQueryDescription> {
		throw new Error("Method not implemented.");
	}
	sqlParameterOptions(
		_appId: string,
		_payload: Pick<ExecuteSqlPayload, "surface" | "overlay_id"> &
			SqlParameterOptionsRequest,
		_userScoped?: boolean,
	): Promise<SqlParameterOptions> {
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
