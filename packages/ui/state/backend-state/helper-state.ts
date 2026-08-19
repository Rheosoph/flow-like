import type { IFileMetadata } from "../../lib";
import type { BulkUploadProgressCallback } from "../../lib/bulk-upload";
import type { ITemporaryUploadResult } from "../../lib/temporary-upload-batch";

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

/** Where the workflow consuming a temporary upload will execute. */
export type ITemporaryUploadExecutionTarget = "local" | "remote";

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
	 * @param executionTarget Where the workflow consuming the upload will run.
	 */
	fileToUrl(
		file: File,
		offline?: boolean,
		appId?: string,
		executionTarget?: ITemporaryUploadExecutionTarget,
	): Promise<string>;

	/**
	 * Uploads a file to temporary storage and returns the URL plus optional FlowPath metadata.
	 * Desktop callers can use executionTarget to keep local runs on a local asset path.
	 */
	fileToTemporaryFile?(
		file: File,
		offline?: boolean,
		appId?: string,
		executionTarget?: ITemporaryUploadExecutionTarget,
	): Promise<ITemporaryUploadedFile>;

	/**
	 * Bulk variant of {@link fileToTemporaryFile}: presigns in chunks and uploads
	 * with bounded parallelism, so folder selections with thousands of files do not
	 * turn into one round trip per file. Results keep the input order and report
	 * failures per file instead of rejecting the whole batch.
	 */
	filesToTemporaryFiles?(
		files: File[],
		options?: {
			offline?: boolean;
			appId?: string;
			executionTarget?: ITemporaryUploadExecutionTarget;
			onProgress?: BulkUploadProgressCallback;
			signal?: AbortSignal;
		},
	): Promise<ITemporaryUploadResult[]>;
}
