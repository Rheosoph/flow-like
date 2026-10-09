import type { HttpClient } from "./client.js";
import { FlowLikeError } from "./errors.js";
import { appPath } from "./paths.js";
import type {
	DownloadFileOptions,
	FileScope,
	ListFilesOptions,
	ListFilesResult,
	PresignOptions,
	PresignResult,
	SignedFile,
	UploadFileOptions,
} from "./types.js";

const dataPath = (app: string, options?: FileScope) =>
	`${appPath(app)}/data${options?.scope === "user" ? "/user" : ""}`;
function grantFor(
	grants: SignedFile[],
	key: string,
): SignedFile & { url: string } {
	const grant = grants.find((item) => item.prefix === key);
	if (!grant?.url || grant.error)
		throw new FlowLikeError(
			grant?.error ?? `No signed URL returned for ${key}`,
		);
	const url = new URL(grant.url);
	if (
		!["https:", "http:"].includes(url.protocol) ||
		url.username ||
		url.password
	)
		throw new FlowLikeError("Invalid signed transfer URL");
	return grant as SignedFile & { url: string };
}
async function transfer(url: string, init: RequestInit): Promise<Response> {
	// Platform credentials belong only on API requests. Signed URLs carry their own authorization.
	const response = await fetch(url, {
		...init,
		redirect: "error",
		credentials: "omit",
	});
	if (!response.ok)
		throw new FlowLikeError(
			`File transfer failed: HTTP ${response.status}`,
			response.status,
		);
	return response;
}
export function createFileMethods(http: HttpClient) {
	const uploadUrls = (
		appId: string,
		files: { prefix: string; size: number }[],
		options?: FileScope,
	) =>
		http.request<SignedFile[]>("PUT", dataPath(appId, options), {
			body: {
				prefixes: files.map((f) => f.prefix),
				sizes: files.map((f) => f.size),
			},
			signal: options?.signal,
		});
	const downloadUrls = (
		appId: string,
		prefixes: string[],
		options?: FileScope,
	) =>
		http.request<SignedFile[]>("POST", `${dataPath(appId, options)}/download`, {
			body: { prefixes },
			signal: options?.signal,
		});
	return {
		listFiles(
			appId: string,
			options?: ListFilesOptions,
		): Promise<ListFilesResult> {
			return http.request("POST", `${dataPath(appId, options)}/list`, {
				body: { prefix: options?.prefix ?? "" },
				query: { refresh: options?.refresh },
				signal: options?.signal,
			});
		},
		getUploadUrls: uploadUrls,
		getDownloadUrls: downloadUrls,
		async uploadFile(
			appId: string,
			file: Blob | File | Uint8Array,
			options?: UploadFileOptions,
		): Promise<SignedFile> {
			const key =
				options?.key ??
				(typeof File !== "undefined" && file instanceof File
					? file.name
					: undefined);
			if (!key)
				throw new FlowLikeError(
					"uploadFile requires options.key or a named File",
				);
			const blob =
				file instanceof Blob ? file : new Blob([new Uint8Array(file)]);
			const grant = grantFor(
				await uploadUrls(appId, [{ prefix: key, size: blob.size }], options),
				key,
			);
			if (grant.method === "POST") {
				if (!grant.fields)
					throw new FlowLikeError("Signed POST upload has no form fields");
				const form = new FormData();
				for (const [name, value] of Object.entries(grant.fields))
					form.append(name, value);
				form.append("file", blob, key);
				await transfer(grant.url, {
					method: "POST",
					body: form,
					signal: options?.signal,
				});
			} else if (
				(grant.method === undefined || grant.method === "PUT") &&
				Object.keys(grant.fields ?? {}).length === 0
			) {
				await transfer(grant.url, {
					method: "PUT",
					body: blob,
					signal: options?.signal,
					headers: options?.contentType
						? { "Content-Type": options.contentType }
						: undefined,
				});
			} else {
				throw new FlowLikeError("Unsupported signed upload method or fields");
			}
			return grant;
		},
		async downloadFile(
			appId: string,
			key: string,
			options?: DownloadFileOptions,
		): Promise<Response> {
			const grant = grantFor(await downloadUrls(appId, [key], options), key);
			return transfer(grant.url, { method: "GET", signal: options?.signal });
		},
		async deleteFile(
			appId: string,
			key: string,
			options?: FileScope,
		): Promise<void> {
			await http.request("DELETE", dataPath(appId, options), {
				body: { prefixes: [key] },
				signal: options?.signal,
			});
		},
		async deleteFiles(
			appId: string,
			prefixes: string[],
			options?: FileScope,
		): Promise<void> {
			await http.request("DELETE", dataPath(appId, options), {
				body: { prefixes },
				signal: options?.signal,
			});
		},
		presignData(
			appId: string,
			options: PresignOptions = {},
		): Promise<PresignResult> {
			return http.request("POST", `${dataPath(appId, options)}/presign`, {
				body: {
					prefix: options.prefix,
					access_mode: options.access_mode ?? "read",
				},
				signal: options.signal,
			});
		},
	};
}
