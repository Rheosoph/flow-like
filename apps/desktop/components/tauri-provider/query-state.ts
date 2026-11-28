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
} from "@flow-like/flow-like-ui";
import { asArray, isRecord } from "@flow-like/flow-like-ui/lib/response-shape";
import { invoke } from "@tauri-apps/api/core";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

function appendScope(url: string, userScoped?: boolean): string {
	if (!userScoped) return url;
	return url.includes("?") ? `${url}&scope=user` : `${url}?scope=user`;
}

/** Result grids render `columns` and `rows` directly, so a hosted reply must carry both. */
export function normalizeExecuteSqlResult(
	result: ExecuteSqlResult,
	source: string,
): ExecuteSqlResult {
	if (!isRecord(result)) {
		throw new Error(
			`${source} returned ${typeof result} instead of a query result`,
		);
	}
	return {
		...result,
		columns: asArray(result.columns),
		rows: asArray(result.rows),
	};
}

function normalizeCrossAppSqlResult(
	result: CrossAppSqlResult,
	source: string,
): CrossAppSqlResult {
	return {
		...result,
		...normalizeExecuteSqlResult(result, source),
		tables: asArray(result.tables),
		cached: result.cached === true,
		computed_at:
			typeof result.computed_at === "number" ? result.computed_at : Date.now(),
		expires_at:
			typeof result.expires_at === "number" ? result.expires_at : null,
	};
}

export class QueryState implements IQueryState {
	constructor(private readonly backend: TauriBackend) {}

	private requireProfile() {
		const profile = this.backend.profile;
		if (!profile) throw new Error("Select a profile before using Data Studio.");
		return profile;
	}

	async listForecasts(
		appId: string,
		userScoped?: boolean,
		maximum = 1000,
	): Promise<StudioForecastSummary[]> {
		if (!Number.isInteger(maximum) || maximum < 1 || maximum > 1000)
			throw new Error("Prediction list size must be between 1 and 1000.");
		const offline = await this.backend.isOffline(appId);
		const forecasts = new Map<string, StudioForecastSummary>();
		for (let offset = 0; offset < maximum; offset += 100) {
			const limit = Math.min(100, maximum - offset);
			const page = offline
				? await invoke<StudioForecastSummary[]>("studio_forecast_list", {
						appId,
						userScoped: userScoped ?? false,
						limit,
						offset,
					})
				: await fetcher<StudioForecastSummary[]>(
						this.requireProfile(),
						appendScope(
							`apps/${encodeURIComponent(appId)}/db/forecasts?limit=${limit}&offset=${offset}`,
							userScoped,
						),
						{ method: "GET" },
						this.backend.auth,
					);
			for (const forecast of asArray(page))
				forecasts.set(forecast.id, forecast);
			if (asArray(page).length < limit) break;
		}
		return [...forecasts.values()];
	}

	async getForecast(
		appId: string,
		forecastId: string,
		userScoped?: boolean,
	): Promise<SavedStudioForecast> {
		if (!(await this.backend.isOffline(appId))) {
			return fetcher<SavedStudioForecast>(
				this.requireProfile(),
				appendScope(
					`apps/${encodeURIComponent(appId)}/db/forecasts/${encodeURIComponent(forecastId)}`,
					userScoped,
				),
				{ method: "GET" },
				this.backend.auth,
			);
		}
		return invoke<SavedStudioForecast>("studio_forecast_get", {
			appId,
			forecastId,
			userScoped: userScoped ?? false,
		});
	}

	async createForecast(
		appId: string,
		payload: CreateStudioForecastPayload,
		userScoped?: boolean,
	): Promise<SavedStudioForecast> {
		if (!(await this.backend.isOffline(appId))) {
			return fetcher<SavedStudioForecast>(
				this.requireProfile(),
				appendScope(
					`apps/${encodeURIComponent(appId)}/db/forecasts`,
					userScoped,
				),
				{ method: "POST", body: JSON.stringify(payload) },
				this.backend.auth,
			);
		}
		return invoke<SavedStudioForecast>("studio_forecast_create", {
			appId,
			payload,
			userScoped: userScoped ?? false,
		});
	}

	async deleteForecast(
		appId: string,
		forecastId: string,
		userScoped?: boolean,
	): Promise<void> {
		if (!(await this.backend.isOffline(appId))) {
			await fetcher(
				this.requireProfile(),
				appendScope(
					`apps/${encodeURIComponent(appId)}/db/forecasts/${encodeURIComponent(forecastId)}`,
					userScoped,
				),
				{ method: "DELETE" },
				this.backend.auth,
			);
			return;
		}
		await invoke("studio_forecast_delete", {
			appId,
			forecastId,
			userScoped: userScoped ?? false,
		});
	}

	async forecast(
		appId: string,
		payload: StudioForecastPayload,
		userScoped?: boolean,
	): Promise<StudioForecastResult> {
		if (!(await this.backend.isOffline(appId))) {
			const profile = this.backend.profile;
			if (!profile) throw new Error("Select a profile before forecasting.");
			return fetcher<StudioForecastResult>(
				profile,
				appendScope(`apps/${appId}/db/queries/forecast`, userScoped),
				{ method: "POST", body: JSON.stringify(payload) },
				this.backend.auth,
			);
		}
		return invoke<StudioForecastResult>("query_forecast", {
			appId,
			payload,
			userScoped: userScoped ?? false,
		});
	}

	async executeSql(
		appId: string,
		payload: ExecuteSqlPayload,
		userScoped?: boolean,
	): Promise<ExecuteSqlResult> {
		const isOffline = await this.backend.isOffline(appId);
		if (!isOffline) {
			return normalizeExecuteSqlResult(
				await fetcher<ExecuteSqlResult>(
					this.requireProfile(),
					appendScope(`apps/${appId}/db/queries/execute`, userScoped),
					{ method: "POST", body: JSON.stringify(payload) },
					this.backend.auth,
				),
				`SQL execution for app ${appId}`,
			);
		}
		return await invoke<ExecuteSqlResult>("query_execute_sql", {
			appId,
			payload,
			userScoped: userScoped ?? false,
		});
	}

	async executeCrossAppSql(
		payload: CrossAppSqlPayload,
	): Promise<CrossAppSqlResult> {
		const local = await Promise.all(
			payload.sources.map((source) => this.backend.isLocalOnly(source.app_id)),
		);
		if (local.length > 0 && local.every(Boolean))
			return normalizeCrossAppSqlResult(
				await invoke<CrossAppSqlResult>("query_execute_cross_app", {
					payload,
				}),
				"The local cross-app query",
			);
		if (local.some(Boolean))
			throw new Error(
				"A query can combine local-only apps or online apps, but not both. Query local-only apps in a separate widget.",
			);
		return normalizeCrossAppSqlResult(
			await fetcher<CrossAppSqlResult>(
				this.requireProfile(),
				"user/data/query",
				{ method: "POST", body: JSON.stringify(payload) },
				this.backend.auth,
			),
			"The cross-app query",
		);
	}

	private async routeCrossApp<T>(
		sources: CrossAppSqlPayload["sources"],
		local: () => Promise<T>,
		remote: () => Promise<T>,
	): Promise<T> {
		const flags = await Promise.all(
			sources.map((source) => this.backend.isLocalOnly(source.app_id)),
		);
		if (flags.length > 0 && flags.every(Boolean)) return local();
		if (flags.some(Boolean))
			throw new Error(
				"A query can combine local-only apps or online apps, but not both. Query local-only apps in a separate widget.",
			);
		return remote();
	}

	async describeCrossAppSql(
		payload: Pick<CrossAppSqlPayload, "sql" | "sources">,
	): Promise<SqlQueryDescription> {
		return this.routeCrossApp(
			payload.sources,
			() =>
				invoke<SqlQueryDescription>("query_describe_cross_app", { payload }),
			() =>
				fetcher<SqlQueryDescription>(
					this.requireProfile(),
					"user/data/query/describe",
					{ method: "POST", body: JSON.stringify(payload) },
					this.backend.auth,
				),
		);
	}

	async crossAppSqlParameterOptions(
		payload: Pick<CrossAppSqlPayload, "sources"> & SqlParameterOptionsRequest,
	): Promise<SqlParameterOptions> {
		return this.routeCrossApp(
			payload.sources,
			() =>
				invoke<SqlParameterOptions>("query_cross_app_parameter_options", {
					payload,
				}),
			() =>
				fetcher<SqlParameterOptions>(
					this.requireProfile(),
					"user/data/query/options",
					{ method: "POST", body: JSON.stringify(payload) },
					this.backend.auth,
				),
		);
	}

	async describeSql(
		appId: string,
		payload: Pick<ExecuteSqlPayload, "sql" | "surface" | "overlay_id">,
		userScoped?: boolean,
	): Promise<SqlQueryDescription> {
		if (!(await this.backend.isOffline(appId)))
			return fetcher<SqlQueryDescription>(
				this.requireProfile(),
				appendScope(`apps/${appId}/db/queries/describe`, userScoped),
				{ method: "POST", body: JSON.stringify(payload) },
				this.backend.auth,
			);
		return invoke<SqlQueryDescription>("query_describe_sql", {
			appId,
			payload,
			userScoped: userScoped ?? false,
		});
	}

	async sqlParameterOptions(
		appId: string,
		payload: Pick<ExecuteSqlPayload, "surface" | "overlay_id"> &
			SqlParameterOptionsRequest,
		userScoped?: boolean,
	): Promise<SqlParameterOptions> {
		if (!(await this.backend.isOffline(appId)))
			return fetcher<SqlParameterOptions>(
				this.requireProfile(),
				appendScope(`apps/${appId}/db/queries/options`, userScoped),
				{ method: "POST", body: JSON.stringify(payload) },
				this.backend.auth,
			);
		return invoke<SqlParameterOptions>("query_sql_parameter_options", {
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
			return asArray(
				await fetcher<SavedQuery[]>(
					this.requireProfile(),
					appendScope(`apps/${appId}/db/queries`, userScoped),
					{ method: "GET" },
					this.backend.auth,
				),
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
				this.requireProfile(),
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
				this.requireProfile(),
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
				this.requireProfile(),
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
				this.requireProfile(),
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
