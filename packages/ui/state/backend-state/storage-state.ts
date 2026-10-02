import type { BulkUploadProgressCallback, IStorageItem } from "../../lib";
import type { IStorageItemActionResult } from "./types";

export interface IStorageUploadOptions {
	/** Cancels an in-flight bulk upload. Ignored by backends that cannot abort. */
	readonly signal?: AbortSignal;
}

export interface IStorageListOptions {
	/** Bypass the server listing cache after a mutation or explicit refresh. */
	readonly refresh?: boolean;
}

export interface IStorageState {
	listStorageItems(
		appId: string,
		prefix: string,
		options?: IStorageListOptions,
	): Promise<IStorageItem[]>;
	listStorageItemsUser(
		appId: string,
		prefix: string,
		options?: IStorageListOptions,
	): Promise<IStorageItem[]>;
	deleteStorageItems(appId: string, prefixes: string[]): Promise<void>;
	deleteStorageItemsUser(appId: string, prefixes: string[]): Promise<void>;
	downloadStorageItems(
		appId: string,
		prefixes: string[],
	): Promise<IStorageItemActionResult[]>;
	downloadStorageItemsUser(
		appId: string,
		prefixes: string[],
	): Promise<IStorageItemActionResult[]>;
	uploadStorageItems(
		appId: string,
		prefix: string,
		files: File[],
		onProgress?: BulkUploadProgressCallback,
		options?: IStorageUploadOptions,
	): Promise<void>;
	uploadStorageItemsUser(
		appId: string,
		prefix: string,
		files: File[],
		onProgress?: BulkUploadProgressCallback,
		options?: IStorageUploadOptions,
	): Promise<void>;
	writeStorageItems?(items: IStorageItemActionResult[]): Promise<void>;
}
