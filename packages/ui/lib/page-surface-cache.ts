import type { Surface } from "../components/a2ui/types";
import type { IPage } from "../state/backend-state/page-state";
import { pageLocalState } from "./idb-storage";

type CacheablePage =
	| Pick<IPage, "id" | "updatedAt" | "cache">
	| null
	| undefined;
type PageSurfaceCacheRecord = {
	pageUpdatedAt: string;
	surface: Surface;
	cachedAt: string;
};

const PAGE_SURFACE_CACHE_KEY = "surface-cache";

function getLegacyCacheKey(appId: string, pageId: string): string {
	return `page-cache:${appId}:${pageId}`;
}

function getLegacyVersionedCacheKey(
	appId: string | undefined,
	page: CacheablePage,
): string | null {
	if (!appId || !page?.id) {
		return null;
	}

	return `page-cache:${appId}:${page.id}:${page.updatedAt}`;
}

function readLegacyPageSurfaceCache(
	appId: string | undefined,
	page: CacheablePage,
): Surface | null {
	if (typeof window === "undefined") {
		return null;
	}

	const versionedKey = getLegacyVersionedCacheKey(appId, page);
	if (!versionedKey || !appId || !page?.id) {
		return null;
	}

	try {
		const cached =
			sessionStorage.getItem(versionedKey) ??
			sessionStorage.getItem(getLegacyCacheKey(appId, page.id));
		return cached ? (JSON.parse(cached) as Surface) : null;
	} catch {
		return null;
	}
}

function clearLegacyPageSurfaceCache(
	appId: string | undefined,
	page: CacheablePage,
): void {
	if (typeof window === "undefined" || !appId || !page?.id) {
		return;
	}

	const versionedKey = getLegacyVersionedCacheKey(appId, page);
	try {
		if (versionedKey) {
			sessionStorage.removeItem(versionedKey);
		}
		sessionStorage.removeItem(getLegacyCacheKey(appId, page.id));
	} catch {
		// sessionStorage unavailable
	}
}

export async function readPageSurfaceCache(
	appId: string | undefined,
	page: CacheablePage,
): Promise<Surface | null> {
	if (!appId || !page?.cache || !page.id) {
		return null;
	}

	try {
		const cachedRecord = await pageLocalState.get<PageSurfaceCacheRecord>(
			appId,
			page.id,
			PAGE_SURFACE_CACHE_KEY,
		);

		if (cachedRecord?.pageUpdatedAt === page.updatedAt) {
			clearLegacyPageSurfaceCache(appId, page);
			return cachedRecord.surface;
		}

		if (cachedRecord) {
			await pageLocalState.delete(appId, page.id, PAGE_SURFACE_CACHE_KEY);
		}

		const legacySurface = readLegacyPageSurfaceCache(appId, page);
		if (legacySurface) {
			await writePageSurfaceCache(appId, page, legacySurface);
			clearLegacyPageSurfaceCache(appId, page);
			return legacySurface;
		}

		return null;
	} catch {
		return null;
	}
}

export async function writePageSurfaceCache(
	appId: string | undefined,
	page: CacheablePage,
	surface: Surface,
): Promise<void> {
	if (!appId || !page?.cache || !page.id) {
		return;
	}

	try {
		await pageLocalState.set(appId, page.id, PAGE_SURFACE_CACHE_KEY, {
			pageUpdatedAt: page.updatedAt,
			surface,
			cachedAt: new Date().toISOString(),
		} satisfies PageSurfaceCacheRecord);
		clearLegacyPageSurfaceCache(appId, page);
	} catch {
		// IndexedDB unavailable
	}
}
