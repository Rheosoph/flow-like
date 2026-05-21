import type { IAppRouteState } from "@flow-like/flow-like-ui";
import type { IRouteMapping } from "@flow-like/flow-like-ui/state/backend-state/route-state";
import {
	type WebBackendRef,
	apiDelete,
	apiGet,
	apiPost,
	apiPut,
} from "./api-utils";

// Generate a simple ID for routes
function generateRouteId(): string {
	return `route_${Date.now()}_${Math.random().toString(36).substring(2, 9)}`;
}

export class WebRouteState implements IAppRouteState {
	constructor(private readonly backend: WebBackendRef) {}

	async getRoutes(appId: string, _force?: boolean): Promise<IRouteMapping[]> {
		try {
			const routes = await apiGet<any[]>(
				`apps/${appId}/routes`,
				this.backend.auth,
			);
			return routes.map((r) => ({
				path: r.path,
				eventId: r.eventId ?? r.event_id,
			}));
		} catch {
			return [];
		}
	}

	async getRouteByPath(
		appId: string,
		path: string,
	): Promise<IRouteMapping | null> {
		try {
			const route = await apiGet<any>(
				`apps/${appId}/routes/by-path?path=${encodeURIComponent(path)}`,
				this.backend.auth,
			);
			if (!route) return null;
			return {
				path: route.path,
				eventId: route.eventId ?? route.event_id,
			};
		} catch {
			return null;
		}
	}

	async getDefaultRoute(appId: string): Promise<IRouteMapping | null> {
		try {
			const route = await apiGet<any>(
				`apps/${appId}/routes/default`,
				this.backend.auth,
			);
			if (!route) return null;
			return {
				path: route.path,
				eventId: route.eventId ?? route.event_id,
			};
		} catch {
			return null;
		}
	}

	async setRoute(
		appId: string,
		path: string,
		eventId: string,
	): Promise<IRouteMapping> {
		// First, check if a route already exists for this path
		try {
			const existingRoute = await apiGet<any>(
				`apps/${appId}/routes/by-path?path=${encodeURIComponent(path)}`,
				this.backend.auth,
			);

			if (existingRoute?.id) {
				// Update existing route
				const result = await apiPut<any>(
					`apps/${appId}/routes/${existingRoute.id}`,
					{
						eventId,
						targetType: "Event",
					},
					this.backend.auth,
				);
				return {
					path: result.path,
					eventId: result.eventId ?? result.event_id,
				};
			}
		} catch {
			// Route doesn't exist, create new one
		}

		// Create new route
		const result = await apiPost<any>(
			`apps/${appId}/routes`,
			{
				id: generateRouteId(),
				path,
				targetType: "Event",
				eventId,
				isDefault: path === "/",
			},
			this.backend.auth,
		);
		return {
			path: result.path,
			eventId: result.eventId ?? result.event_id,
		};
	}

	async setRoutes(
		appId: string,
		routes: Record<string, string>,
	): Promise<IRouteMapping[]> {
		const results: IRouteMapping[] = [];
		for (const [path, eventId] of Object.entries(routes)) {
			const result = await this.setRoute(appId, path, eventId);
			results.push(result);
		}
		return results;
	}

	async deleteRouteByPath(appId: string, path: string): Promise<void> {
		// First get the route to find its ID
		try {
			const route = await apiGet<any>(
				`apps/${appId}/routes/by-path?path=${encodeURIComponent(path)}`,
				this.backend.auth,
			);
			if (route?.id) {
				await apiDelete(`apps/${appId}/routes/${route.id}`, this.backend.auth);
			}
		} catch {
			// Route may not exist
		}
	}

	async deleteRouteByEvent(appId: string, eventId: string): Promise<void> {
		// Get all routes and delete the one matching the event
		try {
			const routes = await apiGet<any[]>(
				`apps/${appId}/routes`,
				this.backend.auth,
			);
			const route = routes.find((r) => (r.eventId ?? r.event_id) === eventId);
			if (route?.id) {
				await apiDelete(`apps/${appId}/routes/${route.id}`, this.backend.auth);
			}
		} catch {
			// Route may not exist
		}
	}
}
