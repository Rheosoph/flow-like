import Dexie, { type EntityTable } from "dexie";

export interface IHomeWidgetParamsRecord {
	/** `${viewerKey}:${widgetId}` */
	id: string;
	values: Record<string, unknown>;
	updatedAt: number;
}

export const HOME_WIDGET_PARAMS_DB = "flow-like-home-widget-params";

let database:
	| (Dexie & { overrides: EntityTable<IHomeWidgetParamsRecord, "id"> })
	| null = null;

function open() {
	if (!database) {
		const created = new Dexie(HOME_WIDGET_PARAMS_DB) as Dexie & {
			overrides: EntityTable<IHomeWidgetParamsRecord, "id">;
		};
		created.version(1).stores({ overrides: "&id, updatedAt" });
		database = created;
	}
	return database;
}

export function homeWidgetParamsKey(
	viewerKey: string | undefined,
	widgetId: string,
): string {
	return `${viewerKey ?? "anonymous"}:${widgetId}`;
}

export async function readHomeWidgetParams(
	key: string,
): Promise<Record<string, unknown>> {
	try {
		return (await open().overrides.get(key))?.values ?? {};
	} catch {
		return {};
	}
}

export async function writeHomeWidgetParams(
	key: string,
	values: Record<string, unknown>,
): Promise<void> {
	try {
		if (Object.keys(values).length === 0) await open().overrides.delete(key);
		else await open().overrides.put({ id: key, values, updatedAt: Date.now() });
	} catch {
		// Storage can be blocked or unavailable; choices then last for the session only.
	}
}
