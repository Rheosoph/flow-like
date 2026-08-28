/**
 * Reading a stored string as the file it points at.
 *
 * Tables keep uploads as paths, not as blobs, so a cell holding
 * `apps/{app}/upload/parsed/page.jpeg` is a document the storage APIs can open —
 * but only for the app that is on screen. Same two-gate shape as the temporal and
 * user cells: a value gate that says what the string looks like, and a name gate
 * for the cases the value alone cannot settle.
 */

import { splitNameSegments } from "./utils";

export type StorageFileScope = "app" | "user";

export interface StorageFileRef {
	/** Which storage root the path is relative to. */
	scope: StorageFileScope;
	/** Path relative to that root, which is what every storage API takes. */
	path: string;
	/** Folder the file sits in, relative to the same root ("" at the root). */
	directory: string;
	fileName: string;
	extension: string;
}

/** Trailing words that make a column a stored file rather than free text. */
const FILE_SUFFIXES = new Set([
	"file",
	"files",
	"filename",
	"filepath",
	"path",
	"paths",
	"key",
	"location",
	"attachment",
	"attachments",
	"asset",
	"assets",
	"document",
	"doc",
	"docs",
	"image",
	"images",
	"img",
	"photo",
	"picture",
	"thumbnail",
	"thumb",
	"media",
	"upload",
	"uploads",
	"artifact",
	"artifacts",
	"source",
]);

/** Leading words that do the same, for `fileRef` and `image_id`. */
const FILE_PREFIXES = new Set([
	"file",
	"image",
	"img",
	"photo",
	"picture",
	"thumbnail",
	"thumb",
	"attachment",
	"asset",
	"document",
	"media",
	"upload",
]);

/**
 * Whether a column name promises a stored file.
 *
 * Anchored to the first and last word like `looksLikeTemporalName`, so
 * `key_value`, `path_segments` and `source_code` stay text.
 */
export function looksLikeFileColumnName(name: string): boolean {
	const segments = splitNameSegments(name);
	if (segments.length === 0) return false;
	return (
		FILE_SUFFIXES.has(segments[segments.length - 1]) ||
		FILE_PREFIXES.has(segments[0])
	);
}

/** Longest path the object store accepts; anything longer is prose. */
const MAX_PATH_LENGTH = 1024;
const FILE_NAME_PATTERN = /^(.+)\.([A-Za-z0-9]{1,8})$/;
/** `http:`, `data:`, `C:` — a scheme or a drive points outside app storage. */
const SCHEME_PATTERN = /^[a-zA-Z][a-zA-Z0-9+.-]*:/;

function readPath(value: unknown): string | null {
	if (typeof value !== "string") return null;
	const trimmed = value.trim();
	if (!trimmed || trimmed.length > MAX_PATH_LENGTH) return null;
	if (/[\n\r\t]/.test(trimmed)) return null;
	if (SCHEME_PATTERN.test(trimmed)) return null;
	// Storage paths are relative and never walk; both forms would only ever 404.
	if (trimmed.startsWith("/") || trimmed.startsWith("\\")) return null;
	if (trimmed.includes("..")) return null;
	return trimmed;
}

function toRef(
	scope: StorageFileScope,
	segments: string[],
	fileName: string,
	extension: string,
): StorageFileRef {
	return {
		scope,
		path: segments.join("/"),
		directory: segments.slice(0, -1).join("/"),
		fileName,
		extension,
	};
}

/**
 * The file a cell points at, or null when it points at nothing openable.
 *
 * A value that names a storage root outright is a file whatever the column is
 * called — `apps/{app}/upload/…` is unmistakable — but the app id in it has to be
 * the app on screen, because a key naming another app resolves back into this
 * app's own root and would silently open the wrong object. Everything else is a
 * bare relative path, believed only where the column name promises a file.
 */
export function resolveStorageFile(
	columnName: string,
	value: unknown,
	appId: string | undefined,
): StorageFileRef | null {
	const raw = readPath(value);
	if (!raw || !appId) return null;

	const segments = raw.split("/").filter(Boolean);
	const fileName = segments[segments.length - 1] ?? "";
	// A path without an extension is as likely to name a folder as a file, and
	// falling back to text is the cheaper mistake.
	const match = FILE_NAME_PATTERN.exec(fileName);
	if (!match) return null;
	const extension = match[2].toLowerCase();

	if (segments[0] === "apps") {
		return segments[1] === appId &&
			segments[2] === "upload" &&
			segments.length > 3
			? toRef("app", segments.slice(3), fileName, extension)
			: null;
	}

	if (segments[0] === "users") {
		return segments[2] === "apps" &&
			segments[3] === appId &&
			segments.length > 4
			? toRef("user", segments.slice(4), fileName, extension)
			: null;
	}

	return looksLikeFileColumnName(columnName)
		? toRef("app", segments, fileName, extension)
		: null;
}
