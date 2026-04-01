import type { IStorageItem, IStorageState } from "@tm9657/flow-like-ui";
import type { IStorageItemActionResult } from "@tm9657/flow-like-ui/state/backend-state/types";
import { type WebBackendRef, apiDelete, apiPost, apiPut } from "./api-utils";

export class WebStorageState implements IStorageState {
	constructor(private readonly backend: WebBackendRef) {}

	private buildFilePath(prefix: string, file: File): string {
		const path =
			(file.webkitRelativePath ?? "") === ""
				? file.name
				: file.webkitRelativePath;
		return prefix ? `${prefix}/${path}` : path;
	}

	private async uploadWithEndpoint(
		endpoint: string,
		prefix: string,
		files: File[],
		onProgress?: (progress: number) => void,
	): Promise<void> {
		const totalFiles = files.length;
		let completedFiles = 0;

		const fileLookup = new Map(
			files.map((file) => [this.buildFilePath(prefix, file), file]),
		);

		const signedUrls = await apiPut<IStorageItemActionResult[]>(
			endpoint,
			{ prefixes: files.map((file) => this.buildFilePath(prefix, file)) },
			this.backend.auth,
		);

		for (const urlInfo of signedUrls) {
			const signedUrl = urlInfo.url;
			if (urlInfo.error || !signedUrl) {
				console.warn(
					`Failed to get signed URL for ${urlInfo.prefix}: ${urlInfo.error}`,
				);
				completedFiles++;
				continue;
			}

			const file = fileLookup.get(urlInfo.prefix);
			if (!file) {
				console.warn(`File not found for prefix: ${urlInfo.prefix}`);
				completedFiles++;
				continue;
			}

			await new Promise<void>((resolve, reject) => {
				const xhr = new XMLHttpRequest();

				xhr.upload.addEventListener("progress", (event) => {
					if (event.lengthComputable) {
						const fileProgress = event.loaded / event.total;
						const totalProgress =
							((completedFiles + fileProgress) / totalFiles) * 100;
						onProgress?.(totalProgress);
					}
				});

				xhr.addEventListener("load", () => {
					if (xhr.status >= 200 && xhr.status < 300) {
						completedFiles++;
						resolve();
					} else {
						reject(
							new Error(
								`Upload failed with status ${xhr.status}: ${xhr.statusText}`,
							),
						);
					}
				});

				xhr.addEventListener("error", () => {
					reject(
						new Error(`Upload failed: Network error (possible CORS issue)`),
					);
				});

				xhr.open("PUT", signedUrl);
				xhr.setRequestHeader(
					"Content-Type",
					file.type || "application/octet-stream",
				);

				if (signedUrl.includes(".blob.core.windows.net")) {
					xhr.setRequestHeader("x-ms-blob-type", "BlockBlob");
				}

				xhr.send(file);
			});
		}

		onProgress?.(100);
	}

	async listStorageItems(
		appId: string,
		prefix: string,
	): Promise<IStorageItem[]> {
		try {
			return await apiPost<IStorageItem[]>(
				`apps/${appId}/data/list`,
				{ prefix },
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async listStorageItemsUser(
		appId: string,
		prefix: string,
	): Promise<IStorageItem[]> {
		try {
			return await apiPost<IStorageItem[]>(
				`apps/${appId}/data/user/list`,
				{ prefix },
				this.backend.auth,
			);
		} catch {
			return [];
		}
	}

	async deleteStorageItems(appId: string, prefixes: string[]): Promise<void> {
		await apiDelete(`apps/${appId}/data`, this.backend.auth, { prefixes });
	}

	async deleteStorageItemsUser(appId: string, prefixes: string[]): Promise<void> {
		await apiDelete(`apps/${appId}/data/user`, this.backend.auth, { prefixes });
	}

	async downloadStorageItems(
		appId: string,
		prefixes: string[],
	): Promise<IStorageItemActionResult[]> {
		try {
			return await apiPost<IStorageItemActionResult[]>(
				`apps/${appId}/data/download`,
				{ prefixes },
				this.backend.auth,
			);
		} catch {
			return prefixes.map((prefix) => ({ prefix, error: "Download failed" }));
		}
	}

	async downloadStorageItemsUser(
		appId: string,
		prefixes: string[],
	): Promise<IStorageItemActionResult[]> {
		try {
			return await apiPost<IStorageItemActionResult[]>(
				`apps/${appId}/data/user/download`,
				{ prefixes },
				this.backend.auth,
			);
		} catch {
			return prefixes.map((prefix) => ({ prefix, error: "Download failed" }));
		}
	}

	async uploadStorageItems(
		appId: string,
		prefix: string,
		files: File[],
		onProgress?: (progress: number) => void,
	): Promise<void> {
		await this.uploadWithEndpoint(`apps/${appId}/data`, prefix, files, onProgress);
	}

	async uploadStorageItemsUser(
		appId: string,
		prefix: string,
		files: File[],
		onProgress?: (progress: number) => void,
	): Promise<void> {
		await this.uploadWithEndpoint(
			`apps/${appId}/data/user`,
			prefix,
			files,
			onProgress,
		);
	}

	async writeStorageItems(items: IStorageItemActionResult[]): Promise<void> {
		// In web mode, items are stored directly on the server
		// This method is primarily for desktop local file writing
	}
}
