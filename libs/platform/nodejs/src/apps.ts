import type { HttpClient, QueryParams } from "./client.js";
import { appPath } from "./paths.js";
import type {
	App,
	HealthResult,
	CreateAppOptions,
	JsonObject,
} from "./types.js";
function parseAppItem(item: unknown): App {
	if (Array.isArray(item)) {
		const data = (item[0] ?? {}) as JsonObject;
		const meta = (item[1] ?? {}) as JsonObject;
		return {
			...data,
			id: String(data.id ?? ""),
			name: String(meta.name ?? data.name ?? ""),
			meta,
		};
	}
	return item as App;
}
export function createAppMethods(http: HttpClient) {
	return {
		async listApps(options: QueryParams = {}): Promise<App[]> {
			return (
				await http.request<unknown[]>("GET", "/apps", { query: options })
			).map(parseAppItem);
		},
		getApp(appId: string): Promise<App> {
			return http.request("GET", appPath(appId));
		},
		createApp(
			nameOrOptions: string | CreateAppOptions,
			description = "",
		): Promise<App> {
			const now = {
				secs_since_epoch: Math.floor(Date.now() / 1000),
				nanos_since_epoch: 0,
			};
			const options =
				typeof nameOrOptions === "string"
					? {
							meta: {
								name: nameOrOptions,
								description,
								tags: [],
								preview_media: [],
								created_at: now,
								updated_at: now,
							},
							bits: [],
						}
					: nameOrOptions;
			return http.request("PUT", "/apps/new", {
				body: { meta: options.meta, bits: options.bits ?? [] },
				query: { language: options.language },
			});
		},
		/** Updates the editable App fields accepted by the server; model manifests are not replaced. */
		updateApp(appId: string, app: JsonObject): Promise<App> {
			return http.request("PUT", appPath(appId), { body: { app } });
		},
		async deleteApp(appId: string): Promise<void> {
			await http.request("DELETE", appPath(appId));
		},
		getAppDetail(appId: string, language?: string): Promise<JsonObject> {
			return http.request("GET", `${appPath(appId)}/detail`, {
				query: { language },
			});
		},
		getAppMeta(appId: string, language?: string): Promise<JsonObject> {
			return http.request("GET", `${appPath(appId)}/meta`, {
				query: { language },
			});
		},
		async updateAppMeta(
			appId: string,
			meta: JsonObject,
			language?: string,
		): Promise<void> {
			await http.request("PUT", `${appPath(appId)}/meta`, {
				body: meta,
				query: { language },
			});
		},
		async setAppVisibility(appId: string, visibility: string): Promise<void> {
			await http.request("PATCH", `${appPath(appId)}/visibility`, {
				body: { visibility },
			});
		},
		forkApp(
			appId: string,
			options: { remote_event_token?: string; language?: string } = {},
		): Promise<JsonObject> {
			return http.request("POST", `${appPath(appId)}/fork`, { body: options });
		},
		listPublicationRequests(appId: string): Promise<JsonObject[]> {
			return http.request("GET", `${appPath(appId)}/publication`);
		},
		requestAppPublication(
			appId: string,
			targetVisibility: string,
			message?: string,
		): Promise<JsonObject> {
			return http.request("POST", `${appPath(appId)}/publication/request`, {
				body: { target_visibility: targetVisibility, message },
			});
		},
		health(): Promise<HealthResult> {
			return http.request("GET", "/health");
		},
	};
}
