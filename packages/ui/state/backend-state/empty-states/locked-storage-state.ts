import {
	type IGetLockedItemRequest,
	type IListLockedItemsRequest,
	type IListLockedItemsResponse,
	type ILockedDownload,
	type ILockedItemDetail,
	type ILockedItemVersion,
	type ILockedStorageState,
	type ILockedStorageStatus,
	LOCKED_STORAGE_DISABLED,
} from "../locked-storage-state";

function unavailable(): never {
	throw new Error("Locked files are not available on this backend.");
}

export class EmptyLockedStorageState implements ILockedStorageState {
	async getLockedStorageStatus(_appId: string): Promise<ILockedStorageStatus> {
		return { ...LOCKED_STORAGE_DISABLED };
	}
	async listLockedItems(
		_appId: string,
		_request: IListLockedItemsRequest,
	): Promise<IListLockedItemsResponse> {
		unavailable();
	}
	async getLockedItem(
		_appId: string,
		_request: IGetLockedItemRequest,
	): Promise<ILockedItemDetail> {
		unavailable();
	}
	async uploadLockedFile(
		_appId: string,
		_file: File,
	): Promise<ILockedItemDetail> {
		unavailable();
	}
	async downloadLockedItem(
		_appId: string,
		_itemId: string,
	): Promise<ILockedDownload> {
		unavailable();
	}
	async setLockedRetention(
		_appId: string,
		_itemId: string,
		_versionId: string,
		_retainUntil: string,
	): Promise<ILockedItemVersion> {
		unavailable();
	}
	async setLockedLegalHold(
		_appId: string,
		_itemId: string,
		_versionId: string,
		_enabled: boolean,
	): Promise<ILockedItemVersion> {
		unavailable();
	}
	async setLockedAutoDelete(
		_appId: string,
		_itemId: string,
		_versionId: string,
		_enabled: boolean,
	): Promise<ILockedItemVersion> {
		unavailable();
	}
	async purgeLockedItem(
		_appId: string,
		_itemId: string,
	): Promise<ILockedItemDetail> {
		unavailable();
	}
}
