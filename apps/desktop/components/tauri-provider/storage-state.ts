import { invoke } from "@tauri-apps/api/core";
import { dirname, join, resolve } from "@tauri-apps/api/path";
import { save } from "@tauri-apps/plugin-dialog";
import { mkdir, open } from "@tauri-apps/plugin-fs";
import type { IStorageItem, IStorageState } from "@tm9657/flow-like-ui";
import type { IStorageItemActionResult } from "@tm9657/flow-like-ui/state/backend-state/types";
import { fetcher, put } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

export class StorageState implements IStorageState {
	constructor(private readonly backend: TauriBackend) {}
	async listStorageItems(
		appId: string,
		prefix: string,
	): Promise<IStorageItem[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (isOffline) {
			const items = await invoke<IStorageItem[]>("storage_list", {
				appId: appId,
				prefix: prefix,
			});
			return items;
		}

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			throw new Error(
				"Backend is not properly initialized for listing storage items.",
			);
		}

		const items = await fetcher<IStorageItem[]>(
			this.backend.profile,
			`apps/${appId}/data/list`,
			{
				method: "POST",
				body: JSON.stringify({
					prefix: prefix,
				}),
			},
			this.backend.auth,
		);

		return items;
	}
	async listStorageItemsUser(
		appId: string,
		prefix: string,
	): Promise<IStorageItem[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (isOffline) {
			const items = await invoke<IStorageItem[]>("storage_user_list", {
				appId: appId,
				prefix: prefix,
			});
			return items;
		}

		if (
			!this.backend.profile ||
			!this.backend.auth ||
			!this.backend.queryClient
		) {
			throw new Error(
				"Backend is not properly initialized for listing storage items.",
			);
		}

		const items = await fetcher<IStorageItem[]>(
			this.backend.profile,
			`apps/${appId}/data/user/list`,
			{
				method: "POST",
				body: JSON.stringify({
					prefix: prefix,
				}),
			},
			this.backend.auth,
		);

		return items;
	}
	async deleteStorageItems(appId: string, prefixes: string[]): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			if (
				!this.backend.profile ||
				!this.backend.auth ||
				!this.backend.queryClient
			) {
				throw new Error("Backend is not properly initialized for deletion.");
			}

			await fetcher<void>(
				this.backend.profile,
				`apps/${appId}/data`,
				{
					method: "DELETE",
					body: JSON.stringify({
						prefixes: prefixes,
					}),
				},
				this.backend.auth,
			);
		}

		await invoke("storage_remove", {
			appId: appId,
			prefixes: prefixes,
		});
	}
	async deleteStorageItemsUser(
		appId: string,
		prefixes: string[],
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			if (
				!this.backend.profile ||
				!this.backend.auth ||
				!this.backend.queryClient
			) {
				throw new Error("Backend is not properly initialized for deletion.");
			}

			await fetcher<void>(
				this.backend.profile,
				`apps/${appId}/data/user`,
				{
					method: "DELETE",
					body: JSON.stringify({
						prefixes: prefixes,
					}),
				},
				this.backend.auth,
			);
		}

		await invoke("storage_user_remove", {
			appId: appId,
			prefixes: prefixes,
		});
	}
	async downloadStorageItems(
		appId: string,
		prefixes: string[],
	): Promise<IStorageItemActionResult[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			if (
				!this.backend.profile ||
				!this.backend.auth ||
				!this.backend.queryClient
			) {
				throw new Error("Backend is not properly initialized for deletion.");
			}

			const files = await fetcher<IStorageItemActionResult[]>(
				this.backend.profile,
				`apps/${appId}/data/download`,
				{
					method: "POST",
					body: JSON.stringify({
						prefixes: prefixes,
					}),
				},
				this.backend.auth,
			);

			return files;
		}

		console.dir({
			isOffline: isOffline,
			profile: this.backend.profile,
			auth: this.backend.auth,
			queryClient: this.backend.queryClient,
			appId: appId,
		});

		const items = await invoke<IStorageItemActionResult[]>("storage_get", {
			appId: appId,
			prefixes: prefixes,
		});
		return items;
	}

	async downloadStorageItemsUser(
		appId: string,
		prefixes: string[],
	): Promise<IStorageItemActionResult[]> {
		const isOffline = await this.backend.isOffline(appId);

		if (!isOffline) {
			if (
				!this.backend.profile ||
				!this.backend.auth ||
				!this.backend.queryClient
			) {
				throw new Error("Backend is not properly initialized for deletion.");
			}

			const files = await fetcher<IStorageItemActionResult[]>(
				this.backend.profile,
				`apps/${appId}/data/user/download`,
				{
					method: "POST",
					body: JSON.stringify({
						prefixes: prefixes,
					}),
				},
				this.backend.auth,
			);

			return files;
		}

		const items = await invoke<IStorageItemActionResult[]>("storage_user_get", {
			appId: appId,
			prefixes: prefixes,
		});
		return items;
	}

	async uploadStorageItems(
		appId: string,
		prefix: string,
		files: File[],
		onProgress?: (progress: number) => void,
	): Promise<void> {
		let totalFiles = files.length;
		let completedFiles = 0;
		console.dir(files);
		console.log(prefix);

		const yieldControl = () => new Promise((resolve) => setTimeout(resolve, 0));

		const batchSize = 2;
		const batches = [];
		for (let i = 0; i < files.length; i += batchSize) {
			batches.push(files.slice(i, i + batchSize));
		}

		const isOffline = await this.backend.isOffline(appId);
		const promises = [] as Promise<void>[];

		if (!isOffline) {
			if (
				!this.backend.profile ||
				!this.backend.auth ||
				!this.backend.queryClient
			) {
				throw new Error("Backend is not properly initialized for deletion.");
			}

			totalFiles = files.length * 2;
			const fileLookup = new Map(
				files.map((file) => {
					const path =
						(file.webkitRelativePath ?? "") === ""
							? file.name
							: file.webkitRelativePath;
					const filePath = `${prefix}/${path}`;
					return [filePath, file];
				}),
			);
			const urls: IStorageItemActionResult[] = await put(
				this.backend.profile,
				`apps/${appId}/data`,
				{
					prefixes: files.map((file) => {
						const path =
							(file.webkitRelativePath ?? "") === ""
								? file.name
								: file.webkitRelativePath;
						return `${prefix}/${path}`;
					}),
				},
				this.backend.auth,
			);

			for (const url of urls) {
				const file = fileLookup.get(url.prefix);
				if (!file) {
					console.warn(`File not found for URL: ${url.prefix}`);
					continue;
				}

				console.group("Uploading file to storage");
				console.dir({
					appId: appId,
					prefix: url.prefix,
					size: file.size,
				});
				console.groupEnd();

				if (url.url)
					promises.push(
						this.backend.uploadSignedUrl(
							url.url,
							file,
							completedFiles,
							totalFiles,
							onProgress,
						),
					);
			}

			await Promise.all(promises);
			return;
		}

		for (const batch of batches) {
			await Promise.all(
				batch.map(async (file) => {
					let filePath = file.name;

					if (file.webkitRelativePath && file.webkitRelativePath !== "") {
						filePath = file.webkitRelativePath;
					}

					filePath = await join(prefix, filePath);

					console.group("Uploading file to storage");
					console.dir({
						appId: appId,
						prefix: filePath,
						size: file.size,
					});
					console.groupEnd();

					const url = await invoke<string>("storage_add", {
						appId: appId,
						prefix: filePath,
					});

					if (
						url.startsWith("asset://") ||
						url.startsWith("http://asset.localhost/")
					) {
						const rawPath = decodeURIComponent(
							url
								.replace("http://asset.localhost/", "")
								.replaceAll("asset://localhost/", ""),
						);

						const parentDir = await dirname(rawPath);
						await mkdir(parentDir, { recursive: true });
						let fileHandle;

						fileHandle = await open(await resolve(rawPath), {
							append: false,
							create: true,
							write: true,
							truncate: true,
						});

						if (!fileHandle) {
							completedFiles++;
							onProgress?.((completedFiles / totalFiles) * 100);
							return;
						}

						const chunkSize = 8 * 1024 * 1024;
						if (file.size < chunkSize) {
							const bytes = new Uint8Array(await file.arrayBuffer());
							await fileHandle.write(bytes);
							await fileHandle.close();
							completedFiles++;
							onProgress?.((completedFiles / totalFiles) * 100);
							return;
						}

						const stream = file.stream();
						const reader = stream.getReader();
						let bytesWritten = 0;
						let chunkCount = 0;

						try {
							while (true) {
								const { done, value } = await reader.read();

								if (done) {
									break;
								}

								await fileHandle.write(value);
								bytesWritten += value.length;
								chunkCount++;

								// Update progress and yield control every few chunks
								if (chunkCount % 5 === 0) {
									const fileProgress = bytesWritten / file.size;
									const totalProgress =
										((completedFiles + fileProgress) / totalFiles) * 100;
									onProgress?.(totalProgress);

									await yieldControl();
								}
							}

							// Final progress update
							completedFiles++;
							onProgress?.((completedFiles / totalFiles) * 100);
						} finally {
							reader.releaseLock();
							await fileHandle.close();
						}
					} else {
						try {
							await this.backend.uploadSignedUrl(
								url,
								file,
								completedFiles,
								totalFiles,
								onProgress,
							);
							completedFiles++;
							onProgress?.((completedFiles / totalFiles) * 100);
						} catch (error) {
							console.error(`Failed to upload file ${filePath}:`, error);
							completedFiles++;
							onProgress?.((completedFiles / totalFiles) * 100);
							throw error;
						}
					}
				}),
			);

			await yieldControl();
		}
	}

	async uploadStorageItemsUser(
		appId: string,
		prefix: string,
		files: File[],
		onProgress?: (progress: number) => void,
	): Promise<void> {
		let totalFiles = files.length;
		let completedFiles = 0;
		console.dir(files);
		console.log(prefix);

		const yieldControl = () => new Promise((resolve) => setTimeout(resolve, 0));

		const batchSize = 2;
		const batches = [];
		for (let i = 0; i < files.length; i += batchSize) {
			batches.push(files.slice(i, i + batchSize));
		}

		const isOffline = await this.backend.isOffline(appId);
		const promises = [] as Promise<void>[];

		if (!isOffline) {
			if (
				!this.backend.profile ||
				!this.backend.auth ||
				!this.backend.queryClient
			) {
				throw new Error("Backend is not properly initialized for deletion.");
			}

			totalFiles = files.length * 2;
			const fileLookup = new Map(
				files.map((file) => {
					const path =
						(file.webkitRelativePath ?? "") === ""
							? file.name
							: file.webkitRelativePath;
					const filePath = prefix ? `${prefix}/${path}` : path;
					return [filePath, file];
				}),
			);
			const urls: IStorageItemActionResult[] = await put(
				this.backend.profile,
				`apps/${appId}/data/user`,
				{
					prefixes: files.map((file) => {
						const path =
							(file.webkitRelativePath ?? "") === ""
								? file.name
								: file.webkitRelativePath;
						return prefix ? `${prefix}/${path}` : path;
					}),
				},
				this.backend.auth,
			);

			for (const url of urls) {
				const file = fileLookup.get(url.prefix);
				if (!file) {
					console.warn(`File not found for URL: ${url.prefix}`);
					continue;
				}

				console.group("Uploading file to user storage");
				console.dir({
					appId: appId,
					prefix: url.prefix,
					size: file.size,
				});
				console.groupEnd();

				if (url.url)
					promises.push(
						this.backend.uploadSignedUrl(
							url.url,
							file,
							completedFiles,
							totalFiles,
							onProgress,
						),
					);
			}

			await Promise.all(promises);
			return;
		}

		for (const batch of batches) {
			await Promise.all(
				batch.map(async (file) => {
					let filePath = file.name;

					if (file.webkitRelativePath && file.webkitRelativePath !== "") {
						filePath = file.webkitRelativePath;
					}

					filePath = await join(prefix, filePath);

					console.group("Uploading file to user storage");
					console.dir({
						appId: appId,
						prefix: filePath,
						size: file.size,
					});
					console.groupEnd();

					const url = await invoke<string>("storage_user_add", {
						appId: appId,
						prefix: filePath,
					});

					if (
						url.startsWith("asset://") ||
						url.startsWith("http://asset.localhost/")
					) {
						const rawPath = decodeURIComponent(
							url
								.replace("http://asset.localhost/", "")
								.replaceAll("asset://localhost/", ""),
						);

						const parentDir = await dirname(rawPath);
						await mkdir(parentDir, { recursive: true });
						let fileHandle;

						fileHandle = await open(await resolve(rawPath), {
							append: false,
							create: true,
							write: true,
							truncate: true,
						});

						if (!fileHandle) {
							completedFiles++;
							onProgress?.((completedFiles / totalFiles) * 100);
							return;
						}

						const chunkSize = 8 * 1024 * 1024;
						if (file.size < chunkSize) {
							const bytes = new Uint8Array(await file.arrayBuffer());
							await fileHandle.write(bytes);
							await fileHandle.close();
							completedFiles++;
							onProgress?.((completedFiles / totalFiles) * 100);
							return;
						}

						const stream = file.stream();
						const reader = stream.getReader();
						let bytesWritten = 0;
						let chunkCount = 0;

						try {
							while (true) {
								const { done, value } = await reader.read();

								if (done) {
									break;
								}

								await fileHandle.write(value);
								bytesWritten += value.length;
								chunkCount++;

								if (chunkCount % 5 === 0) {
									const fileProgress = bytesWritten / file.size;
									const totalProgress =
										((completedFiles + fileProgress) / totalFiles) * 100;
									onProgress?.(totalProgress);

									await yieldControl();
								}
							}

							completedFiles++;
							onProgress?.((completedFiles / totalFiles) * 100);
						} finally {
							reader.releaseLock();
							await fileHandle.close();
						}
					} else {
						try {
							await this.backend.uploadSignedUrl(
								url,
								file,
								completedFiles,
								totalFiles,
								onProgress,
							);
							completedFiles++;
							onProgress?.((completedFiles / totalFiles) * 100);
						} catch (error) {
							console.error(`Failed to upload file ${filePath}:`, error);
							completedFiles++;
							onProgress?.((completedFiles / totalFiles) * 100);
							throw error;
						}
					}
				}),
			);

			await yieldControl();
		}
	}

	async writeStorageItems(items: IStorageItemActionResult[]) {
		for (const file of items) {
			const path = await save({
				canCreateDirectories: true,
				title: file.prefix.split("/").pop() || "Download File",
				defaultPath: file.prefix.split("/").pop(),
			});

			if (path && file.url) {
				const fileHandle = await open(path, {
					create: true,
					write: true,
					truncate: true,
				});

				const fileStream = await fetch(file.url);
				const reader = fileStream.body?.getReader();
				if (!reader) {
					console.error(`Failed to read file stream for ${file.prefix}`);
					await fileHandle.close();
					continue;
				}

				try {
					while (true) {
						const { done, value } = await reader.read();
						if (done) break;

						await fileHandle.write(value);
					}
				} finally {
					reader.releaseLock();
					await fileHandle.close();
				}
			}
		}
	}
}
