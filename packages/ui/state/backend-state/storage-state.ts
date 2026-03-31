import type { IStorageItem } from "../../lib";
import type { IStorageItemActionResult } from "./types";

export interface IStorageState {
	listStorageItems(appId: string, prefix: string): Promise<IStorageItem[]>;
	listStorageItemsUser(appId: string, prefix: string): Promise<IStorageItem[]>;
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
		onProgress?: (progress: number) => void,
	): Promise<void>;
	uploadStorageItemsUser(
		appId: string,
		prefix: string,
		files: File[],
		onProgress?: (progress: number) => void,
	): Promise<void>;
	writeStorageItems?(items: IStorageItemActionResult[]): Promise<void>;
}
