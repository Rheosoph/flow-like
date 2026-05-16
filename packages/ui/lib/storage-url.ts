const AZURE_BLOB_HOST_SUFFIX = ".blob.core.windows.net";

export function isAzureBlobStorageUrl(value: string): boolean {
	try {
		return new URL(value).hostname
			.toLowerCase()
			.endsWith(AZURE_BLOB_HOST_SUFFIX);
	} catch {
		return false;
	}
}
