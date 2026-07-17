import {
	type IFileMetadata,
	type IHelperState,
	type ITemporaryFlowPath,
	type ITemporaryUploadExecutionTarget,
	type ITemporaryUploadedFile,
	getOrUploadTemporaryFile,
	temporaryFilesDb,
} from "@flow-like/flow-like-ui";
import { createId } from "@paralleldrive/cuid2";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { appCacheDir } from "@tauri-apps/api/path";
import { open } from "@tauri-apps/plugin-dialog";
import { mkdir, writeFile } from "@tauri-apps/plugin-fs";
import { get } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

interface ITemporaryFileResponse {
	key: string;
	flowPath?: ITemporaryFlowPath;
	contentType: string;
	uploadUrl: string;
	uploadExpiresAt: string;
	downloadUrl: string;
	downloadExpiresAt: string;
	headUrl: string;
	deleteUrl: string;
	sizeLimitBytes?: number;
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
			await this.fileToTemporaryFile(
				file,
				offline,
				appId,
				executionTarget,
			)
		).url;
	}

	async fileToTemporaryFile(
		file: File,
		offline = false,
		appId?: string,
		executionTarget?: ITemporaryUploadExecutionTarget,
	): Promise<ITemporaryUploadedFile> {
		const useLocalTemporaryFile =
			executionTarget === "local" ||
			offline ||
			(appId ? await this.backend.isOffline(appId) : false);
		const profileScope =
			this.backend.profile?.id ?? this.backend.profile?.hub ?? "no-profile";
		const scope = `desktop:${profileScope}:${appId ?? "global"}:${useLocalTemporaryFile ? "local" : "remote"}`;

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

			const cacheDir = await appCacheDir();
			const fileId = createId();

			const extension = file.name.split(".").pop();

			try {
				await mkdir(`${cacheDir}/chat`, { recursive: true });
			} catch (e) {}

			const tmpPath = `${cacheDir}/chat/${fileId}.${extension}`;

			await writeFile(tmpPath, file.stream());

			const postProcessedPath = await invoke<string>(
				"post_process_local_file",
				{ file: tmpPath },
			);

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
		});
	}
}

function buildContentDisposition(
	filename: string,
	disposition: "inline" | "attachment" = "inline",
): string {
	// 1. Fallback ASCII filename (for old/strict user agents)
	// - Normalize to decompose accents
	// - Strip non-ASCII
	// - Replace quotes/backslashes
	let fallback = filename
		.normalize("NFKD")
		.replace(/[^\x20-\x7E]+/g, "") // remove non-ASCII
		.replace(/["\\]/g, "_")
		.trim();

	if (!fallback) {
		fallback = "file";
	}

	// 2. RFC 5987 / RFC 6266 UTF-8 filename*
	const encoded = encodeURIComponent(filename);

	return `${disposition}; filename="${fallback}"; filename*=UTF-8''${encoded}`;
}
