/**
 * Entity tags for page reads.
 *
 * The desktop talks to the API through the Tauri HTTP plugin, which keeps no HTTP cache, so a
 * revalidation only collapses into a 304 when the request carries the tag itself. A tag is only
 * usable while the payload it describes is still the one on disk, so each entry records the
 * revision it was issued for and is ignored the moment the local page moves on.
 */

const STORAGE_KEY = "flow-like:page-etags";
/** Page tags are ~70 bytes each; a few hundred stay far below the localStorage budget. */
const MAX_ENTRIES = 300;

interface EtagEntry {
	readonly etag: string;
	/** `updatedAt` of the payload this tag was issued for. */
	readonly revision: string;
	readonly usedAt: number;
}

type EtagMap = Record<string, EtagEntry>;

export function pageEtagKey(
	appId: string,
	pageId: string,
	boardId?: string,
	version?: [number, number, number],
): string {
	return `${appId}:${pageId}:${boardId ?? ""}:${version?.join(".") ?? "latest"}`;
}

function readMap(): EtagMap {
	if (typeof localStorage === "undefined") return {};
	try {
		const raw = localStorage.getItem(STORAGE_KEY);
		if (!raw) return {};
		const parsed = JSON.parse(raw);
		return parsed && typeof parsed === "object" ? (parsed as EtagMap) : {};
	} catch {
		return {};
	}
}

function writeMap(map: EtagMap): void {
	if (typeof localStorage === "undefined") return;
	try {
		localStorage.setItem(STORAGE_KEY, JSON.stringify(map));
	} catch {
		// Quota or private-mode failures only cost a revalidation round trip.
	}
}

function evictOldest(map: EtagMap): EtagMap {
	const entries = Object.entries(map);
	if (entries.length <= MAX_ENTRIES) return map;
	entries.sort((a, b) => b[1].usedAt - a[1].usedAt);
	return Object.fromEntries(entries.slice(0, MAX_ENTRIES));
}

/**
 * Returns a tag only when it was issued for exactly the revision the caller still holds.
 * Anything else — no entry, a page that has since been rewritten — falls back to a full read.
 */
export function readPageEtag(
	key: string,
	revision?: string,
): string | undefined {
	if (!revision) return undefined;
	const entry = readMap()[key];
	if (!entry || entry.revision !== revision) return undefined;
	return entry.etag;
}

export function writePageEtag(
	key: string,
	revision: string | undefined,
	etag: string | undefined,
): void {
	if (!revision || !etag) return;
	const map = readMap();
	map[key] = { etag, revision, usedAt: Date.now() };
	writeMap(evictOldest(map));
}

export function clearPageEtags(): void {
	if (typeof localStorage === "undefined") return;
	try {
		localStorage.removeItem(STORAGE_KEY);
	} catch {
		// Nothing to clean up if storage is unavailable.
	}
}
