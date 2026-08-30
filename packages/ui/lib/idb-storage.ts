import { createStore, del, get, keys, set } from "idb-keyval";

/**
 * IndexedDB storage manager for Flow-Like application state
 * Provides typed key-value storage with namespaced stores
 */

// Create separate stores for different data types
const routesStore = createStore("flow-like-routes", "routes");
const pageStateStore = createStore("flow-like-page-state", "page-state");
const globalStateStore = createStore("flow-like-global-state", "global-state");
const elementValuesStore = createStore(
	"flow-like-element-values",
	"element-values",
);

const UNSAFE_RECORD_KEYS = new Set(["__proto__", "constructor", "prototype"]);

/**
 * The `getAll` helpers rebuild a plain record from stored keys, so a row whose
 * key segment is `__proto__` would replace that record's prototype instead of
 * becoming an entry. Writers have rejected those keys since state validation
 * landed; rows predating it are dropped on read, matching the writers.
 */
function isSafeRecordKey(key: string): boolean {
	return key.length > 0 && !UNSAFE_RECORD_KEYS.has(key);
}

// Route storage helpers
export const routeStorage = {
	async getRoutes(appId: string): Promise<AppRouteRecord[]> {
		const routes = await get<AppRouteRecord[]>(`routes:${appId}`, routesStore);
		return routes ?? [];
	},

	async setRoutes(appId: string, routes: AppRouteRecord[]): Promise<void> {
		await set(`routes:${appId}`, routes, routesStore);
	},

	async getRouteByPath(
		appId: string,
		path: string,
	): Promise<AppRouteRecord | null> {
		const routes = await this.getRoutes(appId);
		return routes.find((r) => r.path === path) ?? null;
	},

	async getDefaultRoute(appId: string): Promise<AppRouteRecord | null> {
		const routes = await this.getRoutes(appId);
		// The "/" path is always the default route
		return routes.find((r) => r.path === "/") ?? null;
	},

	async addRoute(
		appId: string,
		route: AppRouteRecord,
	): Promise<AppRouteRecord> {
		const routes = await this.getRoutes(appId);

		// Check for duplicate path
		if (routes.some((r) => r.path === route.path)) {
			throw new Error("Route with this path already exists");
		}

		routes.push(route);
		await this.setRoutes(appId, routes);
		return route;
	},

	async updateRoute(
		appId: string,
		routeId: string,
		updates: Partial<AppRouteRecord>,
	): Promise<AppRouteRecord> {
		const routes = await this.getRoutes(appId);
		const idx = routes.findIndex((r) => r.id === routeId);

		if (idx === -1) {
			throw new Error("Route not found");
		}

		// Check for duplicate path
		if (
			updates.path &&
			routes.some((r) => r.path === updates.path && r.id !== routeId)
		) {
			throw new Error("Route with this path already exists");
		}

		const updated = {
			...routes[idx],
			...updates,
			updatedAt: new Date().toISOString(),
		};
		routes[idx] = updated;
		await this.setRoutes(appId, routes);
		return updated;
	},

	async deleteRoute(appId: string, routeId: string): Promise<void> {
		const routes = await this.getRoutes(appId);
		const filtered = routes.filter((r) => r.id !== routeId);
		await this.setRoutes(appId, filtered);
	},

	async clearAppRoutes(appId: string): Promise<void> {
		await del(`routes:${appId}`, routesStore);
	},
};

// Page-scoped state (local to current page/route)
export const pageLocalState = {
	async get<T>(appId: string, pageId: string, key: string): Promise<T | null> {
		const data = await get<T>(`${appId}:${pageId}:${key}`, pageStateStore);
		return data ?? null;
	},

	async set<T>(
		appId: string,
		pageId: string,
		key: string,
		value: T,
	): Promise<void> {
		await set(`${appId}:${pageId}:${key}`, value, pageStateStore);
	},

	async delete(appId: string, pageId: string, key: string): Promise<void> {
		await del(`${appId}:${pageId}:${key}`, pageStateStore);
	},

	async getAll(
		appId: string,
		pageId: string,
	): Promise<Record<string, unknown>> {
		const allKeys = await keys(pageStateStore);
		const prefix = `${appId}:${pageId}:`;
		const result: Record<string, unknown> = {};

		for (const k of allKeys) {
			const keyStr = String(k);
			if (keyStr.startsWith(prefix)) {
				const shortKey = keyStr.slice(prefix.length);
				if (!isSafeRecordKey(shortKey)) continue;
				result[shortKey] = await get(k, pageStateStore);
			}
		}

		return result;
	},

	async clearPage(appId: string, pageId: string): Promise<void> {
		const allKeys = await keys(pageStateStore);
		const prefix = `${appId}:${pageId}:`;

		for (const k of allKeys) {
			if (String(k).startsWith(prefix)) {
				await del(k, pageStateStore);
			}
		}
	},
};

// App-scoped global state (shared across all pages in an app)
export const appGlobalState = {
	async get<T>(appId: string, key: string): Promise<T | null> {
		const data = await get<T>(`${appId}:${key}`, globalStateStore);
		return data ?? null;
	},

	async set<T>(appId: string, key: string, value: T): Promise<void> {
		await set(`${appId}:${key}`, value, globalStateStore);
	},

	async delete(appId: string, key: string): Promise<void> {
		await del(`${appId}:${key}`, globalStateStore);
	},

	async getAll(appId: string): Promise<Record<string, unknown>> {
		const allKeys = await keys(globalStateStore);
		const prefix = `${appId}:`;
		const result: Record<string, unknown> = {};

		for (const k of allKeys) {
			const keyStr = String(k);
			if (keyStr.startsWith(prefix)) {
				const shortKey = keyStr.slice(prefix.length);
				if (!isSafeRecordKey(shortKey)) continue;
				result[shortKey] = await get(k, globalStateStore);
			}
		}

		return result;
	},

	async clearApp(appId: string): Promise<void> {
		const allKeys = await keys(globalStateStore);
		const prefix = `${appId}:`;

		for (const k of allKeys) {
			if (String(k).startsWith(prefix)) {
				await del(k, globalStateStore);
			}
		}
	},
};

// App-scoped element values (for input fields, dynamic elements - accessible globally)
export interface ElementValue {
	elementId: string;
	value: unknown;
	updatedAt: string;
}

export const elementValues = {
	async get(appId: string, elementId: string): Promise<ElementValue | null> {
		const data = await get<ElementValue>(
			`${appId}:${elementId}`,
			elementValuesStore,
		);
		return data ?? null;
	},

	async getValue<T>(appId: string, elementId: string): Promise<T | null> {
		const data = await this.get(appId, elementId);
		return (data?.value as T) ?? null;
	},

	async set(appId: string, elementId: string, value: unknown): Promise<void> {
		await set(
			`${appId}:${elementId}`,
			{
				elementId,
				value,
				updatedAt: new Date().toISOString(),
			} satisfies ElementValue,
			elementValuesStore,
		);
	},

	async delete(appId: string, elementId: string): Promise<void> {
		await del(`${appId}:${elementId}`, elementValuesStore);
	},

	async getAll(appId: string): Promise<Record<string, ElementValue>> {
		const allKeys = await keys(elementValuesStore);
		const prefix = `${appId}:`;
		const result: Record<string, ElementValue> = {};

		for (const k of allKeys) {
			const keyStr = String(k);
			if (keyStr.startsWith(prefix)) {
				const elementId = keyStr.slice(prefix.length);
				if (!isSafeRecordKey(elementId)) continue;
				const value = await get<ElementValue>(k, elementValuesStore);
				if (value) {
					result[elementId] = value;
				}
			}
		}

		return result;
	},

	async getAllValues(appId: string): Promise<Record<string, unknown>> {
		const all = await this.getAll(appId);
		const result: Record<string, unknown> = {};
		for (const [elementId, entry] of Object.entries(all)) {
			result[elementId] = entry.value;
		}
		return result;
	},

	async clearApp(appId: string): Promise<void> {
		const allKeys = await keys(elementValuesStore);
		const prefix = `${appId}:`;

		for (const k of allKeys) {
			if (String(k).startsWith(prefix)) {
				await del(k, elementValuesStore);
			}
		}
	},
};

// Type definitions
export interface AppRouteRecord {
	id: string;
	appId: string;
	path: string;
	targetType: "page" | "event";
	pageId?: string;
	boardId?: string;
	pageVersion?: [number, number, number];
	eventId?: string;
	priority: number;
	label?: string;
	icon?: string;
	createdAt: string;
	updatedAt: string;
}
