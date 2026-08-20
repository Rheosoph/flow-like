import {
	type BulkUploadProgressCallback,
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
import { type WebBackendRef, apiDelete, apiFetch, apiPost } from "./api-utils";

export class WebStorageState implements IStorageState {
	constructor(private readonly backend: WebBackendRef) {}

	/**
	 * Uploads run against presigned URLs, which the API mints in batches — see
	 * `MAX_PREFIXES` in `packages/api/src/routes/app/data/upload_files.rs`.
	 * Asking for every URL up front puts a folder's worth of paths into one
	 * request body and exceeds the limit, so the orchestrator resolves them a
	 * batch at a time while transfers for earlier batches are still running.
	 */
	private async uploadWithEndpoint(
		endpoint: string,
		prefix: string,
		files: File[],
		onProgress?: BulkUploadProgressCallback,
		options?: IStorageUploadOptions,
	): Promise<void> {
		const result = await runBulkUpload<string>(
			toUploadTasks(prefix, files),
			{
				prepare: async (paths, signal) => {
					const signed = await apiFetch<IStorageItemActionResult[]>(
						endpoint,
						{
							method: "PUT",
							body: JSON.stringify({ prefixes: paths }),
							signal,
						},
						this.backend.auth,
					);
					const targets = new Map<string, string>();
					for (const entry of signed) {
						if (entry.url) targets.set(entry.prefix, entry.url);
						else if (entry.error) {
							console.warn(
								`Failed to get signed URL for ${entry.prefix}: ${entry.error}`,
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

		assertBulkUploadSucceeded(result);
	}

	// Failures propagate: swallowing them into an empty array makes a denied or
	// broken listing indistinguishable from an empty folder.
	async listStorageItems(
		appId: string,
		prefix: string,
	): Promise<IStorageItem[]> {
		return await apiPost<IStorageItem[]>(
			`apps/${appId}/data/list`,
			{ prefix },
			this.backend.auth,
		);
	}

	async listStorageItemsUser(
		appId: string,
		prefix: string,
	): Promise<IStorageItem[]> {
		return await apiPost<IStorageItem[]>(
			`apps/${appId}/data/user/list`,
			{ prefix },
			this.backend.auth,
		);
	}

	async deleteStorageItems(appId: string, prefixes: string[]): Promise<void> {
		await apiDelete(`apps/${appId}/data`, this.backend.auth, { prefixes });
	}

	async deleteStorageItemsUser(
		appId: string,
		prefixes: string[],
	): Promise<void> {
		await apiDelete(`apps/${appId}/data/user`, this.backend.auth, { prefixes });
	}

	// Batched: the route caps a request at MAX_PREFIXES and now rejects
	// anything larger, and callers (an a2ui surface resolving its images, a
	// multi-file selection) legitimately ask for more than that.
	private downloadWithEndpoint(
		endpoint: string,
		prefixes: string[],
	): Promise<IStorageItemActionResult[]> {
		return requestPrefixesInBatches(
			prefixes,
			async (batch) =>
				stabilizeSignedUrls(
					await apiPost<IStorageItemActionResult[]>(
						endpoint,
						{ prefixes: batch },
						this.backend.auth,
					),
				),
			{ errorMessage: "Download failed" },
		);
	}

	async downloadStorageItems(
		appId: string,
		prefixes: string[],
	): Promise<IStorageItemActionResult[]> {
		return this.downloadWithEndpoint(`apps/${appId}/data/download`, prefixes);
	}

	async downloadStorageItemsUser(
		appId: string,
		prefixes: string[],
	): Promise<IStorageItemActionResult[]> {
		return this.downloadWithEndpoint(
			`apps/${appId}/data/user/download`,
			prefixes,
		);
	}

	async uploadStorageItems(
		appId: string,
		prefix: string,
		files: File[],
		onProgress?: BulkUploadProgressCallback,
		options?: IStorageUploadOptions,
	): Promise<void> {
		await this.uploadWithEndpoint(
			`apps/${appId}/data`,
			prefix,
			files,
			onProgress,
			options,
		);
	}

	async uploadStorageItemsUser(
		appId: string,
		prefix: string,
		files: File[],
		onProgress?: BulkUploadProgressCallback,
		options?: IStorageUploadOptions,
	): Promise<void> {
		await this.uploadWithEndpoint(
			`apps/${appId}/data/user`,
			prefix,
			files,
			onProgress,
			options,
		);
	}

	async writeStorageItems(items: IStorageItemActionResult[]): Promise<void> {
		// In web mode, items are stored directly on the server
		// This method is primarily for desktop local file writing
	}
}
