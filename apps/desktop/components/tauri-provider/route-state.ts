import { invoke } from "@tauri-apps/api/core";
import {
	type IAppRouteState,
	type IRouteMapping,
	injectDataFunction,
} from "@flow-like/flow-like-ui";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

interface RemoteRouteMapping {
	id: string;
	path: string;
	eventId: string;
	isDefault: boolean;
}

function toRouteMapping(r: RemoteRouteMapping): IRouteMapping {
	return { path: r.path, eventId: r.eventId };
}

/**
 * Merge server route data with routes backed by the device's local event
 * catalog. The server wins for a conflicting path, while a local-only path is
 * retained so an incomplete remote mirror cannot hide a Local interface.
 */
export function mergeLocalAndRemoteRoutes(
	localRoutes: IRouteMapping[],
	remoteRoutes: IRouteMapping[],
): IRouteMapping[] {
	const merged = new Map<string, IRouteMapping>();

	for (const route of localRoutes) {
		merged.set(route.path, route);
	}

	for (const route of remoteRoutes) {
		merged.set(route.path, route);
	}

	return Array.from(merged.values()).toSorted((a, b) =>
		a.path.localeCompare(b.path),
	);
}

export class RouteState implements IAppRouteState {
	constructor(private readonly backend: TauriBackend) {}

	private canSync(): boolean {
		return !!this.backend.profile && !!this.backend.auth;
	}

	async getRoutes(appId: string, force?: boolean): Promise<IRouteMapping[]> {
		const local = await invoke<IRouteMapping[]>("get_app_routes", { appId });

		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.canSync() || !this.backend.queryClient) {
			return local;
		}

		const syncRemote = async () => {
			const remote = await fetcher<RemoteRouteMapping[]>(
				this.backend.profile!,
				`apps/${appId}/routes`,
				{ method: "GET" },
				this.backend.auth,
			);
			const mapped = remote.map(toRouteMapping);
			for (const r of mapped) {
				await invoke("set_app_route", {
					appId,
					path: r.path,
					eventId: r.eventId,
				}).catch(() => {});
			}
			return mergeLocalAndRemoteRoutes(local, mapped);
		};

		if (force) {
			try {
				const remoteData = await syncRemote();
				const queryKey = [this.getRoutes.name || "backendFn", appId, true];
				this.backend.queryClient.setQueryData(queryKey, remoteData);
				return remoteData;
			} catch (error) {
				if (local.length === 0) throw error;
				console.warn(
					"[RouteSync] Forced route fetch failed, falling back to local routes:",
					error,
				);
				return local;
			}
		}

		if (local.length === 0) {
			try {
				const remoteData = await syncRemote();
				const queryKey = [this.getRoutes.name || "backendFn", appId];
				this.backend.queryClient.setQueryData(queryKey, remoteData);
				return remoteData;
			} catch {
				return local;
			}
		}

		const promise = injectDataFunction(
			syncRemote,
			this,
			this.backend.queryClient,
			this.getRoutes,
			[appId],
			[],
			local,
		);

		this.backend.backgroundTaskHandler(promise);
		return local;
	}

	async getRouteByPath(
		appId: string,
		path: string,
	): Promise<IRouteMapping | null> {
		const local = await invoke<IRouteMapping | null>("get_app_route_by_path", {
			appId,
			path,
		});

		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.canSync()) {
			return local;
		}

		try {
			const remote = await fetcher<RemoteRouteMapping | null>(
				this.backend.profile!,
				`apps/${appId}/routes/by-path?path=${encodeURIComponent(path)}`,
				{ method: "GET" },
				this.backend.auth,
			);
			const mapped = remote ? toRouteMapping(remote) : null;
			this.backend.queryClient?.setQueryData(
				[this.getRouteByPath.name || "backendFn", appId, path],
				mapped,
			);
			if (mapped) {
				await invoke("set_app_route", {
					appId,
					path: mapped.path,
					eventId: mapped.eventId,
				}).catch(() => {});
			}
			return mapped;
		} catch (error) {
			console.warn(
				"[RouteSync] Route fetch failed, falling back to local route:",
				error,
			);
		}

		return local;
	}

	async getDefaultRoute(appId: string): Promise<IRouteMapping | null> {
		const local = await invoke<IRouteMapping | null>("get_default_app_route", {
			appId,
		});

		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.canSync()) {
			return local;
		}

		try {
			const remote = await fetcher<RemoteRouteMapping | null>(
				this.backend.profile!,
				`apps/${appId}/routes/default`,
				{ method: "GET" },
				this.backend.auth,
			);
			const mapped = remote ? toRouteMapping(remote) : null;
			this.backend.queryClient?.setQueryData(
				[this.getDefaultRoute.name || "backendFn", appId],
				mapped,
			);
			if (mapped) {
				await invoke("set_app_route", {
					appId,
					path: mapped.path,
					eventId: mapped.eventId,
				}).catch(() => {});
			}
			return mapped;
		} catch (error) {
			console.warn(
				"[RouteSync] Default route fetch failed, falling back to local route:",
				error,
			);
		}

		return local;
	}

	async setRoute(
		appId: string,
		path: string,
		eventId: string,
	): Promise<IRouteMapping> {
		const local = await invoke<IRouteMapping>("set_app_route", {
			appId,
			path,
			eventId,
		});

		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.canSync()) return local;

		this.backend.backgroundTaskHandler(
			fetcher<RemoteRouteMapping>(
				this.backend.profile!,
				`apps/${appId}/routes`,
				{
					method: "POST",
					body: JSON.stringify({
						path,
						eventId,
						isDefault: path === "/",
					}),
				},
				this.backend.auth,
			).catch((e) => console.warn("[RouteSync] Failed to sync setRoute:", e)),
		);

		return local;
	}

	async setRoutes(
		appId: string,
		routes: Record<string, string>,
	): Promise<IRouteMapping[]> {
		const mappings = Object.entries(routes).map(([path, eventId]) => ({
			path,
			eventId,
		}));
		const local = await invoke<IRouteMapping[]>("set_app_routes", {
			appId,
			routes: mappings,
		});

		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.canSync()) return local;

		this.backend.backgroundTaskHandler(
			(async () => {
				for (const { path, eventId } of mappings) {
					await fetcher<RemoteRouteMapping>(
						this.backend.profile!,
						`apps/${appId}/routes`,
						{
							method: "POST",
							body: JSON.stringify({
								path,
								eventId,
								isDefault: path === "/",
							}),
						},
						this.backend.auth,
					).catch((e) =>
						console.warn("[RouteSync] Failed to sync setRoutes:", e),
					);
				}
			})(),
		);

		return local;
	}

	async deleteRouteByPath(appId: string, path: string): Promise<void> {
		await invoke("delete_app_route_by_path", { appId, path });

		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.canSync()) return;

		this.backend.backgroundTaskHandler(
			(async () => {
				const remote = await fetcher<RemoteRouteMapping | null>(
					this.backend.profile!,
					`apps/${appId}/routes/by-path?path=${encodeURIComponent(path)}`,
					{ method: "GET" },
					this.backend.auth,
				).catch(() => null);
				if (remote?.id) {
					await fetcher<void>(
						this.backend.profile!,
						`apps/${appId}/routes/${remote.id}`,
						{ method: "DELETE" },
						this.backend.auth,
					).catch((e) =>
						console.warn("[RouteSync] Failed to sync deleteRouteByPath:", e),
					);
				}
			})(),
		);
	}

	async deleteRouteByEvent(appId: string, eventId: string): Promise<void> {
		await invoke("delete_app_route_by_event", { appId, eventId });

		const isOffline = await this.backend.isOffline(appId);
		if (isOffline || !this.canSync()) return;

		this.backend.backgroundTaskHandler(
			fetcher<void>(
				this.backend.profile!,
				`apps/${appId}/routes/${eventId}`,
				{ method: "DELETE" },
				this.backend.auth,
			).catch((e) =>
				console.warn("[RouteSync] Failed to sync deleteRouteByEvent:", e),
			),
		);
	}
}
