/**
 * Single-file transfer to a presigned storage URL.
 *
 * `fetch` cannot report upload progress, so this stays on `XMLHttpRequest`.
 * Every caller previously carried its own copy of this — the web storage
 * backend, the desktop provider — and they disagreed on abort handling and on
 * which failures were distinguishable, which is what made retries impossible to
 * add in one place.
 */

import { BulkUploadAbortError, BulkUploadHttpError } from "./bulk-upload";
import { isAzureBlobStorageUrl } from "./storage-url";

export interface ISignedUploadOptions {
	/** Cumulative bytes sent for this file. */
	onBytes?: (loaded: number, total: number) => void;
	signal?: AbortSignal;
	contentType?: string;
	/** Preserves the original filename on the stored object. */
	contentDisposition?: string;
}

export function uploadToSignedUrl(
	signedUrl: string,
	file: Blob,
	options: ISignedUploadOptions = {},
): Promise<void> {
	const { onBytes, signal, contentType, contentDisposition } = options;

	return new Promise<void>((resolve, reject) => {
		if (signal?.aborted) {
			reject(new BulkUploadAbortError());
			return;
		}

		const xhr = new XMLHttpRequest();
		let settled = false;

		const cleanup = () => {
			signal?.removeEventListener("abort", onAbort);
		};
		const finish = (fn: () => void) => {
			if (settled) return;
			settled = true;
			cleanup();
			fn();
		};
		const onAbort = () => {
			// `xhr.abort()` fires the abort listener below, so reject there and
			// keep a single rejection path.
			xhr.abort();
		};

		xhr.upload.addEventListener("progress", (event) => {
			if (event.lengthComputable) onBytes?.(event.loaded, event.total);
		});

		xhr.addEventListener("load", () => {
			if (xhr.status >= 200 && xhr.status < 300) {
				onBytes?.(file.size, file.size);
				finish(resolve);
				return;
			}
			finish(() =>
				reject(
					new BulkUploadHttpError(
						xhr.status,
						`Upload failed with status ${xhr.status}: ${xhr.statusText || "no status text"}`,
					),
				),
			);
		});

		// Status 0 is the browser refusing to expose the real one — network
		// down, DNS, TLS, or a missing CORS header on the bucket. It is
		// retryable, and a run that fails every file this way is the signature
		// of a bucket without CORS configured for this origin.
		xhr.addEventListener("error", () =>
			finish(() =>
				reject(
					new BulkUploadHttpError(
						0,
						"Upload failed: network error (check storage CORS configuration)",
					),
				),
			),
		);
		xhr.addEventListener("timeout", () =>
			finish(() => reject(new BulkUploadHttpError(408, "Upload timed out"))),
		);
		xhr.addEventListener("abort", () =>
			finish(() => reject(new BulkUploadAbortError())),
		);

		xhr.open("PUT", signedUrl);
		xhr.setRequestHeader(
			"Content-Type",
			contentType || file.type || "application/octet-stream",
		);
		if (contentDisposition) {
			xhr.setRequestHeader(
				isAzureBlobStorageUrl(signedUrl)
					? "x-ms-blob-content-disposition"
					: "Content-Disposition",
				contentDisposition,
			);
		}
		if (isAzureBlobStorageUrl(signedUrl)) {
			xhr.setRequestHeader("x-ms-blob-type", "BlockBlob");
		}

		signal?.addEventListener("abort", onAbort, { once: true });
		xhr.send(file);
	});
}
