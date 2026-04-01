import type { HttpClient } from "./client.js";
import type {
	DownloadFileOptions,
	ListFilesOptions,
	ListFilesResult,
	PresignOptions,
	PresignResult,
	UploadFileOptions,
} from "./types.js";

export function createFileMethods(http: HttpClient) {
	return {
		async listFiles(
			appId: string,
			options?: ListFilesOptions,
		): Promise<ListFilesResult> {
			return http.request<ListFilesResult>("GET", `/apps/${appId}/data/list`, {
				query: {
					prefix: options?.prefix,
					cursor: options?.cursor,
					limit: options?.limit,
				},
			});
		},

		async uploadFile(
			appId: string,
			file: Blob | File | Buffer,
			options?: UploadFileOptions,
		): Promise<unknown> {
			const formData = new FormData();
			const blob = file instanceof Blob ? file : new Blob([file as Buffer]);
			formData.append("file", blob, options?.key);

			const headers: Record<string, string> = {};
			if (options?.contentType) {
				headers["X-Content-Type"] = options.contentType;
			}

			return http.request("POST", `/apps/${appId}/data/upload`, {
				body: formData,
				headers,
			});
		},

		async downloadFile(
			appId: string,
			key: string,
			options?: DownloadFileOptions,
		): Promise<Response> {
			return http.requestRaw("POST", `/apps/${appId}/data/download`, {
				body: { key },
				signal: options?.signal,
			});
		},

		async deleteFile(appId: string, key: string): Promise<void> {
			await http.request("DELETE", `/apps/${appId}/data/delete`, {
				body: { key },
			});
		},

		async presignData(
			appId: string,
			options: PresignOptions,
		): Promise<PresignResult> {
			return http.request<PresignResult>(
				"POST",
				`/apps/${appId}/data/presign`,
				{ body: options },
			);
		},
	};
}
