import { getOrUploadTemporaryFile } from "@flow-like/flow-like-ui";
import type {
	IHelperState,
	ITemporaryFlowPath,
	ITemporaryUploadExecutionTarget,
	ITemporaryUploadedFile,
} from "@flow-like/flow-like-ui";
import type { IFileMetadata } from "@flow-like/flow-like-ui/lib";
import { type WebBackendRef, apiGet } from "./api-utils";

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
}

function buildContentDisposition(
	filename: string,
	disposition: "inline" | "attachment" = "inline",
): string {
	let fallback = filename
		.normalize("NFKD")
		.replace(/[^\x20-\x7E]+/g, "")
		.replace(/["\\]/g, "_")
		.trim();

	if (!fallback) {
		fallback = "file";
	}

	const encoded = encodeURIComponent(filename);
	return `${disposition}; filename="${fallback}"; filename*=UTF-8''${encoded}`;
}
