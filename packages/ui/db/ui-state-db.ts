import Dexie, { type EntityTable } from "dexie";

/**
 * Dexie-based UI State Database
 * Stores element values and UI state for the application
 */

export interface IUIElementValue {
	id: string; // `${appId}:${elementId}`
	appId: string;
	elementId: string;
	value: unknown;
	updatedAt: number;
}

export interface IUIPageState {
	id: string; // `${appId}:${pageId}:${key}`
	appId: string;
	pageId: string;
	key: string;
	value: unknown;
	updatedAt: number;
}

export interface IUIGlobalState {
	id: string; // `${appId}:${key}`
	appId: string;
	key: string;
	value: unknown;
	updatedAt: number;
}

const uiStateDb = new Dexie("UI-State-DB") as Dexie & {
	elementValues: EntityTable<IUIElementValue, "id">;
	pageState: EntityTable<IUIPageState, "id">;
	globalState: EntityTable<IUIGlobalState, "id">;
};

uiStateDb.version(1).stores({
	elementValues: "&id, appId, elementId, updatedAt",
	pageState: "&id, appId, pageId, key, updatedAt",
	globalState: "&id, appId, key, updatedAt",
});

export { uiStateDb };

export function elementValueKey(appId: string, elementId: string): string {
	return `${appId}:${elementId}`;
}

export function pageStateKey(
	appId: string,
	pageId: string,
	key: string,
): string {
	return `${appId}:${pageId}:${key}`;
}

export function globalStateKey(appId: string, key: string): string {
	return `${appId}:${key}`;
}

const UNSAFE_RECORD_KEYS = new Set(["__proto__", "constructor", "prototype"]);

/**
 * The `getAll` helpers rebuild a plain record keyed by stored ids, so a row
 * whose id is `__proto__` would replace that record's prototype instead of
 * becoming an entry. Writers reject those ids; rows predating that are dropped
 * on read, matching the writers.
 */
function isSafeRecordKey(key: string): boolean {
	return key.length > 0 && !UNSAFE_RECORD_KEYS.has(key);
}

export const uiElementValues = {
	async get(
		appId: string,
		elementId: string,
	): Promise<IUIElementValue | undefined> {
		return uiStateDb.elementValues.get(elementValueKey(appId, elementId));
	},

	async getValue<T>(appId: string, elementId: string): Promise<T | undefined> {
		const record = await this.get(appId, elementId);
		return record?.value as T | undefined;
	},

	async set(appId: string, elementId: string, value: unknown): Promise<void> {
		await uiStateDb.elementValues.put({
			id: elementValueKey(appId, elementId),
			appId,
			elementId,
			value,
			updatedAt: Date.now(),
		});
	},

	async delete(appId: string, elementId: string): Promise<void> {
		await uiStateDb.elementValues.delete(elementValueKey(appId, elementId));
	},

	async getAll(appId: string): Promise<Record<string, IUIElementValue>> {
		const records = await uiStateDb.elementValues
			.where("appId")
			.equals(appId)
			.toArray();

		const result: Record<string, IUIElementValue> = {};
		for (const record of records) {
			if (!isSafeRecordKey(record.elementId)) continue;
			result[record.elementId] = record;
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
		await uiStateDb.elementValues.where("appId").equals(appId).delete();
	},
};

/**
 * Drops element values older than `maxAgeMs`.
 *
 * Nothing ever deleted these, so a long-lived install accumulated a record for every input any
 * page ever had. They are only useful while they still describe what the user was doing, which
 * makes age the natural bound.
 */
export async function pruneElementValues(
	appId: string,
	maxAgeMs: number,
): Promise<void> {
	const cutoff = Date.now() - maxAgeMs;
	await uiStateDb.elementValues
		.where("appId")
		.equals(appId)
		.filter((record) => record.updatedAt < cutoff)
		.delete();
}

export const uiPageState = {
	async get<T>(
		appId: string,
		pageId: string,
		key: string,
	): Promise<T | undefined> {
		const record = await uiStateDb.pageState.get(
			pageStateKey(appId, pageId, key),
		);
		return record?.value as T | undefined;
	},

	async set<T>(
		appId: string,
		pageId: string,
		key: string,
		value: T,
	): Promise<void> {
		await uiStateDb.pageState.put({
			id: pageStateKey(appId, pageId, key),
			appId,
			pageId,
			key,
			value,
			updatedAt: Date.now(),
		});
	},

	async delete(appId: string, pageId: string, key: string): Promise<void> {
		await uiStateDb.pageState.delete(pageStateKey(appId, pageId, key));
	},

	async getAll(
		appId: string,
		pageId: string,
	): Promise<Record<string, unknown>> {
		const records = await uiStateDb.pageState
			.where("appId")
			.equals(appId)
			.filter((r) => r.pageId === pageId)
			.toArray();

		const result: Record<string, unknown> = {};
		for (const record of records) {
			if (!isSafeRecordKey(record.key)) continue;
			result[record.key] = record.value;
		}
		return result;
	},

	async clearPage(appId: string, pageId: string): Promise<void> {
		await uiStateDb.pageState
			.where("appId")
			.equals(appId)
			.filter((r) => r.pageId === pageId)
			.delete();
	},
};

export const uiGlobalState = {
	async get<T>(appId: string, key: string): Promise<T | undefined> {
		const record = await uiStateDb.globalState.get(globalStateKey(appId, key));
		return record?.value as T | undefined;
	},

	async set<T>(appId: string, key: string, value: T): Promise<void> {
		await uiStateDb.globalState.put({
			id: globalStateKey(appId, key),
			appId,
			key,
			value,
			updatedAt: Date.now(),
		});
	},

	async delete(appId: string, key: string): Promise<void> {
		await uiStateDb.globalState.delete(globalStateKey(appId, key));
	},

	async getAll(appId: string): Promise<Record<string, unknown>> {
		const records = await uiStateDb.globalState
			.where("appId")
			.equals(appId)
			.toArray();

		const result: Record<string, unknown> = {};
		for (const record of records) {
			if (!isSafeRecordKey(record.key)) continue;
			result[record.key] = record.value;
		}
		return result;
	},

	async clearApp(appId: string): Promise<void> {
		await uiStateDb.globalState.where("appId").equals(appId).delete();
	},
};
