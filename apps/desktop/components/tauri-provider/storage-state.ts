import {
	BulkUploadAbortError,
	type BulkUploadProgressCallback,
	type IBulkUploadResult,
	type IStorageItem,
	type IStorageState,
	type IStorageUploadOptions,
	assertBulkUploadSucceeded,
	requestPrefixesInBatches,
	runBulkUpload,
	toUploadTasks,
	uploadToSignedUrl,
} from "@flow-like/flow-like-ui";
import { stabilizeSignedUrls } from "@flow-like/flow-like-ui/lib/stable-asset-url";
import type { IStorageItemActionResult } from "@flow-like/flow-like-ui/state/backend-state/types";
import { invoke } from "@tauri-apps/api/core";
import { dirname, resolve } from "@tauri-apps/api/path";
import { save } from "@tauri-apps/plugin-dialog";
import { mkdir, open, remove } from "@tauri-apps/plugin-fs";
import { fetcher } from "../../lib/api";
import type { TauriBackend } from "../tauri-provider";

type StorageScope = "app" | "user";

/**
 * Local writes go through Tauri IPC to the filesystem, so they do not benefit
 * from the connection parallelism a remote upload does. Kept low deliberately.
 */
const LOCAL_WRITE_CONCURRENCY = 4;

/** Files at or above this size stream instead of being buffered whole. */
const LOCAL_STREAM_THRESHOLD_BYTES = 8 * 1024 * 1024;

function isLocalAssetUrl(url: string): boolean {
	return (
		url.startsWith("asset://") || url.startsWith("http://asset.localhost/")
	);
}

/**
 * Write one file to the path behind an `asset://` URL, streaming anything large
 * enough that buffering it would spike memory.
 */
async function writeLocalFile(
	url: string,
	file: File,
	onBytes: (loaded: number) => void,
	signal: AbortSignal,
): Promise<void> {
	const rawPath = decodeURIComponent(
		url
			.replace("http://asset.localhost/", "")
			.replaceAll("asset://localhost/", ""),
	);

	const parentDir = await dirname(rawPath);
	await mkdir(parentDir, { recursive: true });

	const resolvedPath = await resolve(rawPath);
	const fileHandle = await open(resolvedPath, {
		append: false,
		create: true,
		write: true,
		truncate: true,
	});
	if (!fileHandle) {
		throw new Error(`Could not open ${rawPath} for writing`);
	}

	let complete = false;
	try {
		if (file.size < LOCAL_STREAM_THRESHOLD_BYTES) {
			await fileHandle.write(new Uint8Array(await file.arrayBuffer()));
			onBytes(file.size);
			complete = true;
			return;
		}

		const reader = file.stream().getReader();
		let bytesWritten = 0;
		try {
			for (;;) {
				if (signal.aborted) throw new BulkUploadAbortError();
				const { done, value } = await reader.read();
				if (done) break;
				await fileHandle.write(value);
				bytesWritten += value.length;
				onBytes(bytesWritten);
			}
		} finally {
			reader.releaseLock();
		}
		complete = true;
	} finally {
		await fileHandle.close();
		// `truncate` already emptied any file that was here, so abandoning the
		// write halfway would leave a partial file in place of the original
		// and list it as a normal storage item.
		if (!complete) {
			await remove(resolvedPath).catch(() => {});
		}
	}
}

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

			const profile = this.backend.profile;
			const auth = this.backend.auth;
			// The route caps a request at MAX_PREFIXES and rejects anything
			// larger, so a large selection goes over in batches.
			return requestPrefixesInBatches(
				prefixes,
				async (batch) =>
					stabilizeSignedUrls(
						await fetcher<IStorageItemActionResult[]>(
							profile,
							`apps/${appId}/data/download`,
							{
								method: "POST",
								body: JSON.stringify({ prefixes: batch }),
							},
							auth,
						),
					),
				{ errorMessage: "Download failed" },
			);
		}

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

			const profile = this.backend.profile;
			const auth = this.backend.auth;
			// The route caps a request at MAX_PREFIXES and rejects anything
			// larger, so a large selection goes over in batches.
			return requestPrefixesInBatches(
				prefixes,
				async (batch) =>
					stabilizeSignedUrls(
						await fetcher<IStorageItemActionResult[]>(
							profile,
							`apps/${appId}/data/user/download`,
							{
								method: "POST",
								body: JSON.stringify({ prefixes: batch }),
							},
							auth,
						),
					),
				{ errorMessage: "Download failed" },
			);
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
		onProgress?: BulkUploadProgressCallback,
		options?: IStorageUploadOptions,
	): Promise<void> {
		await this.uploadInternal(appId, prefix, files, "app", onProgress, options);
	}

	async uploadStorageItemsUser(
		appId: string,
		prefix: string,
		files: File[],
		onProgress?: BulkUploadProgressCallback,
		options?: IStorageUploadOptions,
	): Promise<void> {
		await this.uploadInternal(
			appId,
			prefix,
			files,
			"user",
			onProgress,
			options,
		);
	}

	/**
	 * Offline apps write straight to disk through the Tauri storage commands;
	 * hosted apps go through presigned URLs the API mints in capped batches.
	 * Both feed the same orchestrator, so retries, bounded concurrency and
	 * byte-weighted progress behave identically either way — only resolving a
	 * destination and moving the bytes differ.
	 */
	private async uploadInternal(
		appId: string,
		prefix: string,
		files: File[],
		scope: StorageScope,
		onProgress?: BulkUploadProgressCallback,
		options?: IStorageUploadOptions,
	): Promise<void> {
		const isOffline = await this.backend.isOffline(appId);
		const result = isOffline
			? await this.uploadOffline(
					appId,
					prefix,
					files,
					scope,
					onProgress,
					options,
				)
			: await this.uploadSigned(
					appId,
					prefix,
					files,
					scope,
					onProgress,
					options,
				);

		assertBulkUploadSucceeded(result);
	}

	private async uploadSigned(
		appId: string,
		prefix: string,
		files: File[],
		scope: StorageScope,
		onProgress?: BulkUploadProgressCallback,
		options?: IStorageUploadOptions,
	): Promise<IBulkUploadResult> {
		const { profile, auth, queryClient } = this.backend;
		if (!profile || !auth || !queryClient) {
			throw new Error("Backend is not properly initialized for uploading.");
		}

		const endpoint =
			scope === "user" ? `apps/${appId}/data/user` : `apps/${appId}/data`;

		return runBulkUpload<string>(
			toUploadTasks(prefix, files),
			{
				prepare: async (paths, signal) => {
					const signed = await fetcher<IStorageItemActionResult[]>(
						profile,
						endpoint,
						{
							method: "PUT",
							body: JSON.stringify({ prefixes: paths }),
							signal,
						},
						auth,
					);
					const targets = new Map<string, string>();
					for (const entry of signed) {
						if (entry.url) targets.set(entry.prefix, entry.url);
						else if (entry.error) {
							console.warn(
								`Failed to sign upload for ${entry.prefix}: ${entry.error}`,
							);
						}
					}
					return targets;
				},
				send: (signedUrl, task, onBytes, signal) =>
					uploadToSignedUrl(signedUrl, task.file, { onBytes, signal }),
			},
			{ onProgress, signal: options?.signal },
		);
	}

	private async uploadOffline(
		appId: string,
		prefix: string,
		files: File[],
		scope: StorageScope,
		onProgress?: BulkUploadProgressCallback,
		options?: IStorageUploadOptions,
	): Promise<IBulkUploadResult> {
		const command = scope === "user" ? "storage_user_add" : "storage_add";

		// Forward slashes, not the platform separator. `storage_add` hands the
		// prefix to `construct_upload`, which splits on '/' alone
		// (packages/storage/src/files/store.rs), so a Windows `join` result
		// would arrive as one long segment and flatten the folder structure.
		// Sharing the online path's helper also drops one IPC round trip per
		// file, which is what used to stall a large folder before its first
		// byte reached disk.
		return runBulkUpload<string>(
			toUploadTasks(prefix, files),
			{
				prepare: async (paths) => {
					const resolved = await Promise.all(
						paths.map(
							async (path) =>
								[
									path,
									await invoke<string>(command, { appId, prefix: path }),
								] as const,
						),
					);
					return new Map(resolved);
				},
				send: async (url, task, onBytes, signal) => {
					if (isLocalAssetUrl(url)) {
						await writeLocalFile(url, task.file, onBytes, signal);
						return;
					}
					await uploadToSignedUrl(url, task.file, { onBytes, signal });
				},
			},
			{
				onProgress,
				signal: options?.signal,
				concurrency: LOCAL_WRITE_CONCURRENCY,
				// One destination per `invoke`, so a batch buys nothing here and
				// a smaller one starts the first write sooner.
				batchSize: LOCAL_WRITE_CONCURRENCY * 4,
			},
		);
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
