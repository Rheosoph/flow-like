import type { IFileMetadata } from "../../lib";

export interface ITemporaryFlowPath {
	path: string;
	store_ref: string;
	cache_store_ref?: string | null;
}

export interface ITemporaryUploadedFile {
	url: string;
	key?: string;
	contentType?: string;
	flowPath?: ITemporaryFlowPath;
	uploadExpiresAt?: string;
	downloadExpiresAt?: string;
	headUrl?: string;
	deleteUrl?: string;
	sizeLimitBytes?: number;
}

export interface IHelperState {
	getPathMeta(folderPath: string): Promise<IFileMetadata[]>;
	openFileOrFolderMenu(
		multiple: boolean,
		directory: boolean,
		recursive: boolean,
	): Promise<string[] | string | undefined>;

	/**
	 * Converts a file to a URL.
	 * @param file The file to convert.
	 * @param offline Whether to use offline storage (optional).
	 * @param appId Optional app id for app-scoped temporary uploads.
	 */
	fileToUrl(file: File, offline?: boolean, appId?: string): Promise<string>;

	/**
	 * Uploads a file to temporary storage and returns the URL plus optional FlowPath metadata.
	 */
	fileToTemporaryFile?(
		file: File,
		offline?: boolean,
		appId?: string,
	): Promise<ITemporaryUploadedFile>;
}
