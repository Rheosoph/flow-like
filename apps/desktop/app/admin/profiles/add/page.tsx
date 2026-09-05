"use client";

import {
	type ProfileMediaUpload,
	ProfileTemplateEditorPage,
} from "@flow-like/flow-like-ui/components/profile-templates/profile-template-editor";
import { fetch as tauriFetch } from "@tauri-apps/plugin-http";

const uploadMedia: ProfileMediaUpload = async (url, file) => {
	const headers: Record<string, string> = { "Content-Type": file.type };
	if (new URL(url).hostname.endsWith(".blob.core.windows.net"))
		headers["x-ms-blob-type"] = "BlockBlob";
	const response = await tauriFetch(url, {
		method: "PUT",
		body: new Uint8Array(await file.arrayBuffer()),
		headers,
	});
	if (!response.ok)
		throw new Error(`Image upload failed (${response.status}). Try again.`);
};

export default function ProfileEditorPage() {
	return <ProfileTemplateEditorPage uploadMedia={uploadMedia} />;
}
