import { isAzureBlobStorageUrl } from "./storage-url";

export interface MediaUploadResponse {
	signed_url?: string | null;
	avatar_upload_id?: string | null;
	upload_pending?: boolean;
}

/** Wait for the server to validate and publish the converted image. */
export async function completeMediaUpload(
	finalize: () => Promise<{ upload_pending?: boolean }>,
	{
		attempts = 30,
		delayMs = 1000,
	}: { attempts?: number; delayMs?: number } = {},
): Promise<void> {
	for (let attempt = 0; attempt < attempts; attempt++) {
		const result = await finalize();
		if (result.upload_pending !== true) return;
		if (attempt + 1 < attempts) {
			await new Promise((resolve) => setTimeout(resolve, delayMs));
		}
	}
	throw new Error(
		"Image processing is taking longer than expected. Your previous image is still in use. Please try again.",
	);
}

export async function uploadProfileMedia(
	url: string,
	file: File,
): Promise<void> {
	const headers: HeadersInit = { "Content-Type": file.type };
	if (isAzureBlobStorageUrl(url)) headers["x-ms-blob-type"] = "BlockBlob";
	const response = await fetch(url, { method: "PUT", body: file, headers });
	if (!response.ok) {
		throw new Error(
			`Image upload failed (${response.status}). Please try again.`,
		);
	}
}

/** Keep account fields unchanged until the photo has uploaded and can be committed. */
export async function updateAccountWithAvatar(
	data: object,
	avatar: File | undefined,
	request: (body: Record<string, unknown>) => Promise<MediaUploadResponse>,
): Promise<void> {
	if (!avatar) {
		await request({ ...data });
		return;
	}
	const prepared = await request({
		avatar_extension: avatar.name.split(".").pop() || "",
	});
	if (!prepared.signed_url || !prepared.avatar_upload_id) {
		throw new Error(
			"The server did not prepare the image upload. Please try again.",
		);
	}
	await uploadProfileMedia(prepared.signed_url, avatar);
	await completeMediaUpload(() =>
		request({ ...data, avatar_upload_id: prepared.avatar_upload_id }),
	);
}
