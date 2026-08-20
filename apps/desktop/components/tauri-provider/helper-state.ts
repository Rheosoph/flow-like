import {
	type IFileMetadata,
	type IHelperState,
	type ITemporaryFlowPath,
	type ITemporaryUploadExecutionTarget,
	type ITemporaryUploadedFile,
	buildContentDisposition,
	getOrUploadTemporaryFile,
	temporaryFilesDb,
	uploadTemporaryFilesInBatches,
	uploadTemporaryFilesLocally,
} from "@flow-like/flow-like-ui";
import type {
	BulkUploadProgressCallback,
	ITemporaryPresignedUpload,
	ITemporaryUploadResult,
} from "@flow-like/flow-like-ui/lib";
import { createId } from "@paralleldrive/cuid2";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { appCacheDir } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import { mkdir, writeFile } from "@tauri-apps/plugin-fs";
import { get, post } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

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

export class HelperState implements IHelperState {
	constructor(private readonly backend: TauriBackend) {}

	async getPathMeta(path: string): Promise<IFileMetadata[]> {
		return await invoke("get_path_meta", {
			path: path,
		});
	}
	async openFileOrFolderMenu(
		multiple: boolean,
		directory: boolean,
		recursive: boolean,
	): Promise<string[] | string | undefined> {
		return (
			(await open({
				multiple: multiple,
				directory: directory,
				recursive: recursive,
			})) ?? undefined
		);
	}

	async fileToUrl(
		file: File,
		offline = false,
		appId?: string,
		executionTarget?: ITemporaryUploadExecutionTarget,
	): Promise<string> {
		return (
			await this.fileToTemporaryFile(file, offline, appId, executionTarget)
		).url;
	}

	private async useLocalTemporaryFile(
		offline: boolean,
		appId?: string,
		executionTarget?: ITemporaryUploadExecutionTarget,
	): Promise<boolean> {
		return (
			executionTarget === "local" ||
			offline ||
			(appId ? await this.backend.isOffline(appId) : false)
		);
	}

	private temporaryUploadScope(local: boolean, appId?: string): string {
		const profileScope =
			this.backend.profile?.id ?? this.backend.profile?.hub ?? "no-profile";
		return `desktop:${profileScope}:${appId ?? "global"}:${local ? "local" : "remote"}`;
	}

	async fileToTemporaryFile(
		file: File,
		offline = false,
		appId?: string,
		executionTarget?: ITemporaryUploadExecutionTarget,
	): Promise<ITemporaryUploadedFile> {
		const useLocalTemporaryFile = await this.useLocalTemporaryFile(
			offline,
			appId,
			executionTarget,
		);
		const scope = this.temporaryUploadScope(useLocalTemporaryFile, appId);

		return getOrUploadTemporaryFile(file, scope, async () => {
			if (!useLocalTemporaryFile) {
				if (!this.backend.profile || !this.backend.auth) {
					throw new Error("Profile or auth not set");
				}

				const params = new URLSearchParams({
					extension: file.name.split(".").pop() || "",
					filename: file.name,
				});
				if (appId) {
					params.set("appId", appId);
				}

				const response: ITemporaryFileResponse = await get(
					this.backend.profile,
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
			}

			return await writeLocalTemporaryFile(file);
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
		const local = await this.useLocalTemporaryFile(
			options?.offline ?? false,
			appId,
			options?.executionTarget,
		);
		const scope = this.temporaryUploadScope(local, appId);

		if (local) {
			return uploadTemporaryFilesLocally(files, writeLocalTemporaryFile, {
				onProgress: options?.onProgress,
				signal: options?.signal,
			});
		}

		const profile = this.backend.profile;
		const auth = this.backend.auth;
		if (!profile || !auth) throw new Error("Profile or auth not set");

		return uploadTemporaryFilesInBatches(files, {
			scope,
			onProgress: options?.onProgress,
			signal: options?.signal,
			presign: async (batch) => {
				const response = await post<ITemporaryFileBatchResponse>(
					profile,
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

async function writeLocalTemporaryFile(
	file: File,
): Promise<ITemporaryUploadedFile> {
	const cacheDir = await appCacheDir();
	const fileId = createId();
	const extension = file.name.split(".").pop();

	try {
		await mkdir(`${cacheDir}/chat`, { recursive: true });
	} catch (e) {}

	const tmpPath = `${cacheDir}/chat/${fileId}.${extension}`;
	await writeFile(tmpPath, file.stream());

	const postProcessedPath = await invoke<string>("post_process_local_file", {
		file: tmpPath,
	});
	const hash = postProcessedPath.split("/").pop() || fileId;

	await temporaryFilesDb.temporaryFiles.put({
		id: fileId,
		fileName: file.name,
		size: file.size,
		hash: hash,
		createdAt: Date.now(),
	});

	const assetUrl = convertFileSrc(postProcessedPath);
	const separator = assetUrl.includes("?") ? "&" : "?";
	return {
		url: `${assetUrl}${separator}filename=${encodeURIComponent(file.name)}`,
	};
}
