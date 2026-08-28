import {
	basename,
	normalizePrefix,
	parentPrefix,
	resolveAssetPath,
} from "../components/builder/asset-path";
import type { IStorageItem } from "./schema/storage/storage-item";

export type IStorageScope = "app" | "user";

/** The storage root — the prefix the list APIs expect for a top-level listing. */
export const STORAGE_ROOT_PREFIX = "";

const STORAGE_NODE_PREFIX = "storage";

export interface IStorageTreeEntry {
	readonly item: IStorageItem;
	/** Display name — the last path segment. */
	readonly name: string;
	/**
	 * Path relative to the storage root. This — never `item.location` — is what
	 * goes back into list/upload/delete.
	 */
	readonly path: string;
	readonly isFolder: boolean;
	readonly nodeId: string;
}

/**
 * Both backends list with a delimiter, so an item is always an immediate child
 * of the prefix that was listed. Cloud reports the absolute object-store key
 * (`apps/<id>/upload/docs/a.pdf`), desktop the already-relative path
 * (`docs/a.pdf`) — they differ only by a base the client never learns. So the
 * item's root-relative path is "the listed prefix plus the item's own name",
 * derived from the prefix alone and never from the shape of the key. Handing an
 * absolute key back instead makes `construct_upload` fold the base on a second
 * time (`apps/<id>/upload/apps/<id>/upload/...`) and the folder lists empty.
 */
export function normalizeStorageLocation(
	location: string,
	prefix: string,
): string {
	return resolveAssetPath(prefix, location);
}

export function storageItemName(item: Pick<IStorageItem, "location">): string {
	return basename(item.location);
}

export function childPrefix(parent: string, name: string): string {
	const base = normalizePrefix(parent);
	const child = normalizePrefix(name);
	if (!child) return base;
	return base ? `${base}/${child}` : child;
}

export { normalizePrefix, parentPrefix };

/** Every prefix from the root down to and including `prefix`. */
export function storagePrefixTrail(prefix: string): readonly string[] {
	const trail: string[] = [STORAGE_ROOT_PREFIX];
	let current = STORAGE_ROOT_PREFIX;
	for (const segment of normalizePrefix(prefix).split("/").filter(Boolean)) {
		current = current ? `${current}/${segment}` : segment;
		trail.push(current);
	}
	return trail;
}

export function storageNodeId(scope: IStorageScope, prefix: string): string {
	return `${STORAGE_NODE_PREFIX}:${scope}:${normalizePrefix(prefix)}`;
}

export function parseStorageNodeId(
	nodeId: string,
): { readonly scope: IStorageScope; readonly prefix: string } | null {
	const [namespace, scope, ...rest] = nodeId.split(":");
	if (namespace !== STORAGE_NODE_PREFIX) return null;
	if (scope !== "app" && scope !== "user") return null;
	// File names may contain ':' — everything past the scope is the prefix.
	return { scope, prefix: rest.join(":") };
}

export function isStorageFolder(item: Pick<IStorageItem, "is_dir">): boolean {
	return item.is_dir ?? false;
}

export function storageTreeEntry(
	item: IStorageItem,
	prefix: string,
	scope: IStorageScope,
): IStorageTreeEntry {
	const path = normalizeStorageLocation(item.location, prefix);
	return {
		item,
		name: basename(path),
		path,
		isFolder: isStorageFolder(item),
		nodeId: storageNodeId(scope, path),
	};
}

export function compareStorageEntries(
	a: IStorageTreeEntry,
	b: IStorageTreeEntry,
): number {
	if (a.isFolder !== b.isFolder) return a.isFolder ? -1 : 1;
	return a.name.localeCompare(b.name, undefined, {
		numeric: true,
		sensitivity: "base",
	});
}

/** Folders first, then files, each alphabetically — listings arrive unordered. */
export function sortStorageEntries(
	entries: readonly IStorageTreeEntry[],
): readonly IStorageTreeEntry[] {
	return [...entries].sort(compareStorageEntries);
}
