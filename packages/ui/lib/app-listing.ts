import { isRecord } from "./response-shape";
import type { IApp } from "./schema/app/app";
import type { IMetadata } from "./schema/bit/bit";

type AppEntry = [IApp, IMetadata | undefined];

const PAGE_SIZE = 100;

function isAppEntry(entry: unknown): entry is AppEntry {
	if (!Array.isArray(entry)) return false;
	const [app, metadata] = entry;
	return (
		isRecord(app) &&
		typeof app.id === "string" &&
		(metadata === undefined || metadata === null || isRecord(metadata))
	);
}

/** Read the complete membership list; the hub returns at most 100 apps per page. */
export async function fetchAllAppPages(
	fetchPage: (offset: number, limit: number) => Promise<unknown>,
): Promise<AppEntry[]> {
	const apps = new Map<string, AppEntry>();
	for (let offset = 0; ; offset += PAGE_SIZE) {
		const page = await fetchPage(offset, PAGE_SIZE);
		if (!Array.isArray(page) || !page.every(isAppEntry)) {
			throw new Error(`App list at offset ${offset} returned an invalid page`);
		}
		const previousSize = apps.size;
		for (const entry of page) {
			if (!apps.has(entry[0].id)) apps.set(entry[0].id, entry);
		}
		if (page.length < PAGE_SIZE) return [...apps.values()];
		if (apps.size === previousSize) {
			throw new Error(`App list at offset ${offset} repeated an earlier page`);
		}
	}
}
