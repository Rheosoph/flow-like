import type {
	BulkUploadProgressCallback,
	IStorageItem,
	IStorageItemActionResult,
	IStorageState,
	IStorageUploadOptions,
} from "@flow-like/flow-like-ui";

export class EmptyStorageState implements IStorageState {
	listStorageItems(appId: string, prefix: string): Promise<IStorageItem[]> {
		throw new Error("Method not implemented.");
	}
	listStorageItemsUser(appId: string, prefix: string): Promise<IStorageItem[]> {
		throw new Error("Method not implemented.");
	}
	deleteStorageItems(appId: string, prefixes: string[]): Promise<void> {
		throw new Error("Method not implemented.");
	}
	deleteStorageItemsUser(appId: string, prefixes: string[]): Promise<void> {
		throw new Error("Method not implemented.");
	}
	downloadStorageItems(
		appId: string,
		prefixes: string[],
	): Promise<IStorageItemActionResult[]> {
		throw new Error("Method not implemented.");
	}
	downloadStorageItemsUser(
		appId: string,
		prefixes: string[],
	): Promise<IStorageItemActionResult[]> {
		throw new Error("Method not implemented.");
	}
	uploadStorageItems(
		appId: string,
		prefix: string,
		files: File[],
		onProgress?: BulkUploadProgressCallback,
		options?: IStorageUploadOptions,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
	uploadStorageItemsUser(
		appId: string,
		prefix: string,
		files: File[],
		onProgress?: BulkUploadProgressCallback,
		options?: IStorageUploadOptions,
	): Promise<void> {
		throw new Error("Method not implemented.");
	}
}
