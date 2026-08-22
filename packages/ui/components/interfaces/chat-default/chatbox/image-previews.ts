"use client";

import { useEffect, useRef } from "react";

export function isImageFile(file: File) {
	return file.type.startsWith("image/");
}

/**
 * Blob URLs for the image files in `files`, keyed by file identity rather than by list identity:
 * a file keeps one URL for as long as it stays in the list, so a thumbnail has its src on the
 * first paint and surviving previews are never re-decoded when the list changes. A URL is revoked
 * once its file leaves the list, `enabled` goes false, or the component unmounts.
 */
export function useImagePreviewUrls(
	files: File[],
	enabled = true,
): ReadonlyMap<File, string> {
	const cache = useRef(new Map<File, string>());
	const urls = new Map<File, string>();

	if (enabled) {
		for (const file of files) {
			if (!isImageFile(file)) continue;
			let url = cache.current.get(file);
			if (!url) {
				url = URL.createObjectURL(file);
				cache.current.set(file, url);
			}
			urls.set(file, url);
		}
	}

	useEffect(() => {
		for (const [file, url] of cache.current) {
			if (urls.has(file)) continue;
			URL.revokeObjectURL(url);
			cache.current.delete(file);
		}
	});

	useEffect(() => {
		const cached = cache.current;
		return () => {
			for (const url of cached.values()) URL.revokeObjectURL(url);
			cached.clear();
		};
	}, []);

	return urls;
}
