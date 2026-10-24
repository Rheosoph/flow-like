export type LockedScope = "project" | "user";
export type LockedRetentionMode = "governance" | "compliance";
export type LockedEnforcement = "object" | "bucket";
export type LockedVersionStatus = "pending" | "locked" | "purged";
export type LockedStorageProvider = "s3" | "r2" | "gcs" | "azure";
export type LockedIfExists = "error" | "new_version";

export interface ILockedStorageStatus {
	enabled: boolean;
	provider: LockedStorageProvider | null;
	enforcement: LockedEnforcement | null;
	retention_mode: LockedRetentionMode;
	default_retention_days: number;
	min_retention_days: number;
	max_retention_days: number;
	max_file_bytes: number;
	supports_legal_hold: boolean;
	supports_retention_extension: boolean;
}

export interface ILockedItemVersion {
	id: string;
	item_id: string;
	version: number | null;
	status: LockedVersionStatus;
	file_name: string;
	content_type: string;
	size: number;
	sha256: string;
	checksum_verified: boolean;
	provider_version_id: string | null;
	retention_mode: LockedRetentionMode;
	retain_until: string;
	enforcement: LockedEnforcement;
	legal_hold: boolean;
	/** Purged automatically once retention ends and no legal hold remains. */
	delete_after_retention: boolean;
	comment: string | null;
	metadata: Record<string, unknown> | null;
	uploaded_by: string;
	created_at: string;
	locked_at: string | null;
	purged_at: string | null;
	purged_by: string | null;
	/** Locked, retention ended and no legal hold. */
	purgeable: boolean;
}

export interface ILockedItem {
	id: string;
	app_id: string;
	scope: LockedScope;
	owner_id: string | null;
	name: string;
	folder: string;
	content_type: string | null;
	latest_version: number;
	latest_size: number;
	/** Stored (not purged) versions. */
	live_versions: number;
	/** Bytes charged to the storage payer. */
	live_size: number;
	metadata: Record<string, unknown> | null;
	created_by: string;
	created_at: string;
	updated_at: string;
	/** Newest locked version. */
	latest: ILockedItemVersion | null;
}

export interface IListLockedItemsRequest {
	scope?: LockedScope;
	/** `""` is the root; nested folders have no leading or trailing slash. */
	folder?: string;
	recursive?: boolean;
	search?: string;
	limit?: number;
	cursor?: string | null;
}

export interface IListLockedItemsResponse {
	items: ILockedItem[];
	/** Immediate child folder names (display form) of the requested folder. */
	folders: string[];
	next_cursor: string | null;
}

export interface IGetLockedItemRequest {
	item_id?: string;
	scope?: LockedScope;
	folder?: string;
	name?: string;
}

export interface ILockedItemDetail {
	item: ILockedItem;
	/** Newest first; pending versions only for the caller's own uploads. */
	versions: ILockedItemVersion[];
}

export interface ICreateLockedUploadRequest {
	item_id?: string;
	scope?: LockedScope;
	folder?: string;
	name?: string;
	file_name: string;
	content_type?: string;
	size: number;
	sha256: string;
	retention_days?: number;
	/** Project files need an admin, as for any purge. */
	delete_after_retention?: boolean;
	comment?: string;
	metadata?: Record<string, unknown>;
	if_exists?: LockedIfExists;
}

export interface ILockedUploadTicket {
	item: ILockedItem;
	version: ILockedItemVersion;
	upload: {
		url: string;
		method: "PUT";
		/** Signed: sent verbatim, nothing added or dropped. */
		headers: Record<string, string>;
		expires_at: string;
	};
}

export interface ILockedDownload {
	item_id: string;
	version_id: string;
	version: number;
	file_name: string;
	content_type: string;
	size: number;
	sha256: string;
	url: string;
	expires_at: string;
}

export interface ILockedUploadProgress {
	readonly phase: "hashing" | "uploading" | "committing";
	readonly loaded: number;
	readonly total: number;
}

export interface ILockedUploadOptions {
	/** Uploads a new version of this item; scope, folder and name are ignored. */
	itemId?: string;
	scope?: LockedScope;
	folder?: string;
	name?: string;
	retentionDays?: number;
	deleteAfterRetention?: boolean;
	comment?: string;
	metadata?: Record<string, unknown>;
	ifExists?: LockedIfExists;
	onProgress?: (progress: ILockedUploadProgress) => void;
	signal?: AbortSignal;
}

export interface ILockedStorageState {
	getLockedStorageStatus(appId: string): Promise<ILockedStorageStatus>;
	listLockedItems(
		appId: string,
		request: IListLockedItemsRequest,
	): Promise<IListLockedItemsResponse>;
	getLockedItem(
		appId: string,
		request: IGetLockedItemRequest,
	): Promise<ILockedItemDetail>;
	/** Hashes, reserves, transfers and commits one file as a new locked version. */
	uploadLockedFile(
		appId: string,
		file: File,
		options?: ILockedUploadOptions,
	): Promise<ILockedItemDetail>;
	downloadLockedItem(
		appId: string,
		itemId: string,
		version?: number,
	): Promise<ILockedDownload>;
	setLockedRetention(
		appId: string,
		itemId: string,
		versionId: string,
		retainUntil: string,
	): Promise<ILockedItemVersion>;
	setLockedLegalHold(
		appId: string,
		itemId: string,
		versionId: string,
		enabled: boolean,
	): Promise<ILockedItemVersion>;
	/** Retention stays unchanged; needs the right to purge the file. */
	setLockedAutoDelete(
		appId: string,
		itemId: string,
		versionId: string,
		enabled: boolean,
	): Promise<ILockedItemVersion>;
	/** Without `versionId` every version is purged; refused while any is retained. */
	purgeLockedItem(
		appId: string,
		itemId: string,
		versionId?: string,
	): Promise<ILockedItemDetail>;
}

export const LOCKED_STORAGE_DISABLED: Readonly<ILockedStorageStatus> =
	Object.freeze({
		enabled: false,
		provider: null,
		enforcement: null,
		retention_mode: "compliance",
		default_retention_days: 0,
		min_retention_days: 0,
		max_retention_days: 0,
		max_file_bytes: 0,
		supports_legal_hold: false,
		supports_retention_extension: false,
	});
