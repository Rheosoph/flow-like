"use client";

import { useEffect, useState } from "react";

export function isImageFile(file: File) {
	return file.type.startsWith("image/");
}

/**
 * Blob URLs for the image files in `files`, minted once per list rather than per render and
 * revoked when the list changes or the component unmounts.
 */
export function useImagePreviewUrls(files: File[]): ReadonlyMap<File, string> {
	const [urls, setUrls] = useState<ReadonlyMap<File, string>>(() => new Map());

	useEffect(() => {
		const minted = new Map<File, string>();
		for (const file of files) {
			if (isImageFile(file) && !minted.has(file)) {
				minted.set(file, URL.createObjectURL(file));
			}
		}
		setUrls((prev) => (prev.size === 0 && minted.size === 0 ? prev : minted));
		return () => {
			for (const url of minted.values()) URL.revokeObjectURL(url);
		};
	}, [files]);

	return urls;
}
