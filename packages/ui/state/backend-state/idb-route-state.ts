import { type AppRouteRecord, routeStorage } from "../../lib/idb-storage";
import type { IAppRouteState, IRouteMapping } from "./route-state";

/**
 * IndexedDB-backed implementation of IAppRouteState
 * Routes are persisted locally in the browser's IndexedDB
 */
export class IDBRouteState implements IAppRouteState {
	private toRouteMapping(record: AppRouteRecord): IRouteMapping {
		return {
			path: record.path,
			eventId: record.eventId ?? "",
		};
	}

	async getRoutes(appId: string, _force?: boolean): Promise<IRouteMapping[]> {
		const records = await routeStorage.getRoutes(appId);
		return records
			.filter((r) => r.eventId)
			.map((r) => this.toRouteMapping(r))
			.sort((a, b) => a.path.localeCompare(b.path));
	}

	async getRouteByPath(
		appId: string,
		path: string,
	): Promise<IRouteMapping | null> {
		const record = await routeStorage.getRouteByPath(appId, path);
		return record && record.eventId ? this.toRouteMapping(record) : null;
	}

	async getDefaultRoute(appId: string): Promise<IRouteMapping | null> {
		const record = await routeStorage.getDefaultRoute(appId);
		return record && record.eventId ? this.toRouteMapping(record) : null;
	}

	async setRoute(
		appId: string,
		path: string,
		eventId: string,
	): Promise<IRouteMapping> {
		// Try to get existing route for this path
		const existing = await routeStorage.getRouteByPath(appId, path);
		const now = new Date().toISOString();

		if (existing) {
			// Update existing route
			const updated = await routeStorage.updateRoute(appId, existing.id, {
				eventId,
			});
			return this.toRouteMapping(updated);
		}

		// Create new route
		const newRecord: AppRouteRecord = {
			id: `${appId}-${path}`,
			appId,
			path,
			targetType: "event",
			eventId,
			priority: 0,
			createdAt: now,
			updatedAt: now,
		};

		const created = await routeStorage.addRoute(appId, newRecord);
		return this.toRouteMapping(created);
	}

	async setRoutes(
		appId: string,
		routes: Record<string, string>,
	): Promise<IRouteMapping[]> {
		const result: IRouteMapping[] = [];
		for (const [path, eventId] of Object.entries(routes)) {
			const mapping = await this.setRoute(appId, path, eventId);
			result.push(mapping);
		}
		return result;
	}

	async deleteRouteByPath(appId: string, path: string): Promise<void> {
		const record = await routeStorage.getRouteByPath(appId, path);
		if (record) {
			await routeStorage.deleteRoute(appId, record.id);
		}
	}

	async deleteRouteByEvent(appId: string, eventId: string): Promise<void> {
		const routes = await routeStorage.getRoutes(appId);
		const toDelete = routes.filter((r) => r.eventId === eventId);
		for (const route of toDelete) {
			await routeStorage.deleteRoute(appId, route.id);
		}
	}
}
