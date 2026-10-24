import type { TFunction } from "i18next";
import type { IStorageItem } from "../../lib/schema/storage/storage-item";
import {
	decodeStorageSegment,
	encodeStorageSegment,
	storageDisplayName,
} from "../../lib/storage-tree";
import type {
	IListLockedItemsResponse,
	ILockedItem,
	ILockedItemVersion,
	LockedScope,
} from "../../state/backend-state/locked-storage-state";

type LockedRetention = Pick<
	ILockedItemVersion,
	"retain_until" | "delete_after_retention" | "legal_hold"
>;

/** Never a listed storage key, so a locked row cannot collide with a storage file of the same name. */
const LOCKED_ENTRY_ROOT = "locked:";

export interface ILockedStorageEntry extends IStorageItem {
	readonly lockedItem: ILockedItem;
}

export function isLockedEntry(item: IStorageItem): item is ILockedStorageEntry {
	return typeof item.lockedItem === "object" && item.lockedItem !== null;
}

export function lockedScopeFor(storageScopeKey: string): LockedScope {
	return storageScopeKey === "user" ? "user" : "project";
}

/** The browser addresses folders by listed (encoded) segments; the locked API by display names. */
export function lockedFolderForPrefix(prefix: string): string {
	return prefix.split("/").filter(Boolean).map(decodeStorageSegment).join("/");
}

function childLocation(prefix: string, displayName: string): string {
	const base = prefix.replace(/^\/+|\/+$/g, "");
	const segment = encodeStorageSegment(displayName);
	return base ? `${base}/${segment}` : segment;
}

/**
 * Locked folders and items as browser rows. Folders reuse the storage folder's
 * encoded segment, so a locked-only folder and a storage folder of the same
 * name navigate to the same prefix; folders storage already lists are skipped.
 */
export function lockedBrowserEntries(
	prefix: string,
	listing: Pick<IListLockedItemsResponse, "items" | "folders"> | undefined,
	storageItems: readonly IStorageItem[],
): IStorageItem[] {
	if (!listing) return [];
	const storageFolders = new Set(
		storageItems
			.filter((item) => item.is_dir)
			.map((item) => storageDisplayName(item.location).toLowerCase()),
	);
	const folders = (listing.folders ?? [])
		.filter((name) => name && !storageFolders.has(name.toLowerCase()))
		.map(
			(name): IStorageItem => ({
				location: childLocation(prefix, name),
				is_dir: true,
				size: 0,
				last_modified: new Date(0).toISOString(),
			}),
		);
	const items = (listing.items ?? []).map(
		(item): ILockedStorageEntry => ({
			location: `${LOCKED_ENTRY_ROOT}${item.id}/${encodeStorageSegment(item.name)}`,
			is_dir: false,
			size: item.live_size,
			last_modified: item.latest?.locked_at ?? item.updated_at,
			lockedItem: item,
		}),
	);
	return [...folders, ...items];
}

const dateFormatters = new Map<string, Intl.DateTimeFormat>();

export function formatLockedDate(
	value: string | null | undefined,
	language: string,
	withTime = false,
): string {
	if (!value) return "";
	const date = new Date(value);
	if (Number.isNaN(date.getTime())) return value;
	const key = `${language}|${withTime}`;
	let formatter = dateFormatters.get(key);
	if (!formatter) {
		formatter = new Intl.DateTimeFormat(language || undefined, {
			dateStyle: "medium",
			...(withTime ? { timeStyle: "short" } : {}),
		});
		dateFormatters.set(key, formatter);
	}
	return formatter.format(date);
}

export function retentionEnded(retainUntil: string): boolean {
	return new Date(retainUntil).getTime() <= Date.now();
}

/** Null unless the server deletes the version by itself; a legal hold keeps it. */
export function autoDeletionLabel(
	version: LockedRetention,
	language: string,
	t: TFunction<"common">,
): string | null {
	if (!version.delete_after_retention || version.legal_hold) return null;
	const date = formatLockedDate(version.retain_until, language);
	if (!retentionEnded(version.retain_until)) {
		return t("lockedStorage.deletedOn", "Deleted automatically on {{date}}", {
			date,
		});
	}
	return t(
		"lockedStorage.deletionDue",
		"Retention ended {{date}}, deletion pending",
		{ date },
	);
}

export function retentionLabel(
	version: LockedRetention,
	language: string,
	t: TFunction<"common">,
): string {
	const automatic = autoDeletionLabel(version, language, t);
	if (automatic) return automatic;
	const date = formatLockedDate(version.retain_until, language);
	return retentionEnded(version.retain_until)
		? t("lockedStorage.retentionEndedOn", "Retention ended {{date}}", { date })
		: t("lockedStorage.retainedUntil", "Retained until {{date}}", { date });
}

/** `YYYY-MM-DD` of the UTC day after `value`, the earliest date an extension may pick. */
export function nextRetentionDay(value: string): string {
	const date = new Date(value);
	date.setUTCDate(date.getUTCDate() + 1);
	return date.toISOString().slice(0, 10);
}

/** Keeps the current retain-until time of day, so any later day is strictly later. */
export function retentionFromDay(day: string, current: string): string {
	return `${day}T${new Date(current).toISOString().slice(11)}`;
}

export function shortSha256(sha256: string): string {
	return sha256.length > 16
		? `${sha256.slice(0, 8)}…${sha256.slice(-8)}`
		: sha256;
}
