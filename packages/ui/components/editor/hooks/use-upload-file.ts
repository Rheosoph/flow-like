"use client";

import { createId } from "@paralleldrive/cuid2";
import * as React from "react";
import { toast } from "sonner";

import { useBackend } from "../../../state/backend-state";
import {
	normalizeUploadPrefix,
	toStorageUrl,
	useEditorUpload,
} from "../upload-context";

export interface UploadedFile {
	/** Durable `storage://…` reference written into the document. */
	url: string;
	/** Path inside the app's upload area, without the scheme. */
	key: string;
	name: string;
	size: number;
	type: string;
}

const EXTENSION_FALLBACKS: Record<string, string> = {
	"image/jpeg": "jpg",
	"image/png": "png",
	"image/gif": "gif",
	"image/webp": "webp",
	"image/svg+xml": "svg",
	"video/mp4": "mp4",
	"video/webm": "webm",
	"audio/mpeg": "mp3",
	"audio/wav": "wav",
	"application/pdf": "pdf",
};

function extensionFor(file: File): string {
	const fromName = file.name.split(".").pop();
	if (fromName && fromName !== file.name && /^[A-Za-z0-9]{1,8}$/.test(fromName)) {
		return fromName.toLowerCase();
	}
	return EXTENSION_FALLBACKS[file.type] ?? file.type.split("/").pop() ?? "bin";
}

/**
 * Upload editor media into app storage.
 *
 * Replaces the upstream Plate template's uploadthing uploader, which pointed at an endpoint this
 * product never mounted: every upload failed and fell back to `URL.createObjectURL`, so images
 * were written into the saved document as `blob:` URLs that died on reload.
 */
export function useUploadFile() {
	const backend = useBackend();
	const { appId, prefix, scope, onUploaded, onUploadError } = useEditorUpload();

	const [uploadedFile, setUploadedFile] = React.useState<UploadedFile>();
	const [uploadingFile, setUploadingFile] = React.useState<File>();
	const [progress, setProgress] = React.useState(0);
	const [isUploading, setIsUploading] = React.useState(false);

	const uploadFile = React.useCallback(
		async (file: File): Promise<UploadedFile | undefined> => {
			if (!appId) {
				const message =
					"Media upload is not available here — this editor has no app storage configured.";
				toast.error(message);
				onUploadError?.(file.name, message);
				return undefined;
			}

			setIsUploading(true);
			setUploadingFile(file);
			setProgress(0);

			const folder = normalizeUploadPrefix(prefix);
			const filename = `${createId()}.${extensionFor(file)}`;
			const renamed = new File([file], filename, { type: file.type });

			try {
				const upload =
					scope === "user"
						? backend.storageState.uploadStorageItemsUser
						: backend.storageState.uploadStorageItems;

				await upload.call(
					backend.storageState,
					appId,
					folder,
					[renamed],
					(value: number) => setProgress(Math.min(Math.max(value, 0), 100)),
				);

				const result: UploadedFile = {
					url: toStorageUrl(folder, filename),
					key: folder ? `${folder}/${filename}` : filename,
					name: file.name,
					size: file.size,
					type: file.type,
				};

				setUploadedFile(result);
				onUploaded?.({
					url: result.url,
					path: result.key,
					name: result.name,
					size: result.size,
					type: result.type,
				});
				return result;
			} catch (error) {
				const message = getErrorMessage(error);
				toast.error(message);
				onUploadError?.(file.name, message);
				return undefined;
			} finally {
				setProgress(0);
				setIsUploading(false);
				setUploadingFile(undefined);
			}
		},
		[appId, backend.storageState, onUploadError, onUploaded, prefix, scope],
	);

	return {
		isUploading,
		progress,
		uploadedFile,
		uploadFile,
		uploadingFile,
	};
}

export function getErrorMessage(err: unknown): string {
	if (err instanceof Error && err.message) {
		return err.message;
	}
	if (typeof err === "string" && err.length > 0) {
		return err;
	}
	return "Upload failed, please try again.";
}

export function showErrorToast(err: unknown) {
	return toast.error(getErrorMessage(err));
}
