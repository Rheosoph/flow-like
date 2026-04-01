import type { HttpClient } from "./client.js";
import type { App, HealthResult } from "./types.js";

function parseAppItem(item: unknown): App {
	// API returns Vec<(App, Option<Metadata>)> → [[app, meta|null], ...]
	if (Array.isArray(item) && item.length >= 1) {
		const appData = (item[0] ?? {}) as Record<string, unknown>;
		const meta = (item[1] ?? {}) as Record<string, unknown>;
		return {
			id: (appData.id as string) ?? "",
			name: (meta.name as string) ?? (appData.name as string) ?? "",
			meta: meta,
			...appData,
		};
	}
	return item as App;
}

export function createAppMethods(http: HttpClient) {
	return {
		async listApps(): Promise<App[]> {
			const raw = await http.request<unknown[]>("GET", "/apps");
			return raw.map(parseAppItem);
		},

		async getApp(appId: string): Promise<App> {
			return http.request<App>("GET", `/apps/${appId}`);
		},

		async createApp(name: string, description?: string): Promise<App> {
			return http.request<App>("POST", "/apps", {
				body: { name, description },
			});
		},

		async health(): Promise<HealthResult> {
			return http.request<HealthResult>("GET", "/health");
		},
	};
}
