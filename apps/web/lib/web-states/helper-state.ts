import {
	buildContentDisposition,
	getOrUploadTemporaryFile,
	uploadTemporaryFilesInBatches,
} from "@flow-like/flow-like-ui";
import type {
	IHelperState,
	ITemporaryFlowPath,
	ITemporaryUploadExecutionTarget,
	ITemporaryUploadedFile,
} from "@flow-like/flow-like-ui";
import type { IFileMetadata } from "@flow-like/flow-like-ui/lib";
import type {
	BulkUploadProgressCallback,
	ITemporaryPresignedUpload,
	ITemporaryUploadResult,
} from "@flow-like/flow-like-ui/lib";
import { type WebBackendRef, apiGet, apiPost } from "./api-utils";

interface ITemporaryFileResponse {
	key: string;
	flowPath?: ITemporaryFlowPath;
	contentType: string;
	uploadUrl: string;
	uploadExpiresAt: string;
	downloadUrl: string;
	downloadExpiresAt: string;
	headUrl?: string;
	deleteUrl?: string;
	sizeLimitBytes?: number;
}

interface ITemporaryFileBatchResponse {
	files: ITemporaryFileResponse[];
}

export class WebHelperState implements IHelperState {
	constructor(private readonly backend: WebBackendRef) {}

	async getPathMeta(folderPath: string): Promise<IFileMetadata[]> {
		return apiGet<IFileMetadata[]>(
			`helper/path-meta?path=${encodeURIComponent(folderPath)}`,
			this.backend.auth,
		);
	}

	async openFileOrFolderMenu(
		multiple: boolean,
		directory: boolean,
		recursive: boolean,
	): Promise<string[] | string | undefined> {
		// Web version: Cannot open native file dialogs
		// This would need to use HTML file input instead
		console.warn(
			"openFileOrFolderMenu is not available in web mode - use HTML file input",
		);
		return undefined;
	}

	async fileToUrl(
		file: File,
		offline?: boolean,
		appId?: string,
		executionTarget?: ITemporaryUploadExecutionTarget,
	): Promise<string> {
		return (
			await this.fileToTemporaryFile(file, offline, appId, executionTarget)
		).url;
	}

	async fileToTemporaryFile(
		file: File,
		_offline?: boolean,
		appId?: string,
		_executionTarget?: ITemporaryUploadExecutionTarget,
	): Promise<ITemporaryUploadedFile> {
		const profileScope =
			this.backend.profile?.id ?? this.backend.profile?.hub ?? "no-profile";
		const scope = `web:${profileScope}:${appId ?? "global"}:${this.backend.auth ? "auth" : "anon"}`;

		return getOrUploadTemporaryFile(file, scope, async () => {
			if (!this.backend.auth) {
				return { url: URL.createObjectURL(file) };
			}

			const params = new URLSearchParams({
				extension: file.name.split(".").pop() || "",
				filename: file.name,
			});
			if (appId) {
				params.set("appId", appId);
			}

			const response: ITemporaryFileResponse = await apiGet(
				`tmp?${params.toString()}`,
				this.backend.auth,
			);

			const uploadResponse = await fetch(response.uploadUrl, {
				method: "PUT",
				headers: {
					"Content-Type": file.type,
					"Content-Disposition": buildContentDisposition(file.name, "inline"),
				},
				body: file,
			});
			if (!uploadResponse.ok) {
				throw new Error(
					`Temporary file upload failed (${uploadResponse.status} ${uploadResponse.statusText})`,
				);
			}

			return {
				url: response.downloadUrl,
				key: response.key,
				contentType: response.contentType,
				flowPath: response.flowPath,
				uploadExpiresAt: response.uploadExpiresAt,
				downloadExpiresAt: response.downloadExpiresAt,
				headUrl: response.headUrl,
				deleteUrl: response.deleteUrl,
				sizeLimitBytes: response.sizeLimitBytes,
			};
		});
	}

	async filesToTemporaryFiles(
		files: File[],
		options?: {
			offline?: boolean;
			appId?: string;
			executionTarget?: ITemporaryUploadExecutionTarget;
			onProgress?: BulkUploadProgressCallback;
			signal?: AbortSignal;
		},
	): Promise<ITemporaryUploadResult[]> {
		const appId = options?.appId;
		const auth = this.backend.auth;
		const profileScope =
			this.backend.profile?.id ?? this.backend.profile?.hub ?? "no-profile";
		const scope = `web:${profileScope}:${appId ?? "global"}:${auth ? "auth" : "anon"}`;

		if (!auth) {
			return files.map((file) => ({
				file,
				uploaded: { url: URL.createObjectURL(file) },
			}));
		}

		return uploadTemporaryFilesInBatches(files, {
			scope,
			onProgress: options?.onProgress,
			signal: options?.signal,
			presign: async (batch) => {
				const response = await apiPost<ITemporaryFileBatchResponse>(
					"tmp/batch",
					{
						appId,
						files: batch.map((file) => ({
							extension: file.name.split(".").pop() || "",
							contentType: file.type || undefined,
						})),
					},
					auth,
				);
				return (response?.files ?? []).map(
					(entry): ITemporaryPresignedUpload => ({
						uploadUrl: entry.uploadUrl,
						downloadUrl: entry.downloadUrl,
						key: entry.key,
						contentType: entry.contentType,
						flowPath: entry.flowPath,
						uploadExpiresAt: entry.uploadExpiresAt,
						downloadExpiresAt: entry.downloadExpiresAt,
						sizeLimitBytes: entry.sizeLimitBytes,
					}),
				);
			},
		});
	}
}
