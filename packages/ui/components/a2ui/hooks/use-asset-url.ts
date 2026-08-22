"use client";

import { useEffect, useRef, useState } from "react";
import { isExpiredAssetUrl } from "../../../lib/stable-asset-url";
import { useBackend } from "../../../state/backend-state";
import { useActionContext } from "../ActionHandler";

interface AssetUrlCache {
	url: string;
	expiresAt: number;
}

const urlCache = new Map<string, AssetUrlCache>();

const CACHE_DURATION_MS = 30 * 60 * 1000;

/**
 * A signed URL dies with the credential that signed it, which is shorter than
 * whatever deadline the entry was given here. Reuse an entry only while both
 * still hold.
 */
function isUsable(cached: AssetUrlCache | undefined): cached is AssetUrlCache {
	return (
		cached !== undefined &&
		cached.expiresAt > Date.now() &&
		!isExpiredAssetUrl(cached.url)
	);
}

function isValidUrl(url: string): boolean {
	return (
		url.startsWith("http://") ||
		url.startsWith("https://") ||
		url.startsWith("data:") ||
		url.startsWith("tauri://") ||
		url.startsWith("asset://") ||
		url.startsWith("file://")
	);
}

function isLocalFilePath(path: string): boolean {
	// Detect absolute local file paths
	// Unix: /Users/..., /home/..., /tmp/..., etc.
	// Windows: C:\..., D:\..., etc.
	return path.startsWith("/") || /^[A-Za-z]:[/\\]/.test(path);
}

function isStoragePath(path: string): boolean {
	// Storage paths look like: "storage://path/to/file" or just "path/to/file" without protocol
	// Regular URLs start with http(s):// or data:
	// Local file paths start with / or C:\ etc.
	return !isValidUrl(path) && !isLocalFilePath(path);
}

export function useAssetUrl(assetPath: string | undefined): {
	url: string | undefined;
	isLoading: boolean;
	error: string | null;
} {
	const backend = useBackend();
	const { appId } = useActionContext();
	const [url, setUrl] = useState<string | undefined>(undefined);
	const [isLoading, setIsLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const abortControllerRef = useRef<AbortController | null>(null);

	useEffect(() => {
		if (!assetPath) {
			setUrl(undefined);
			setIsLoading(false);
			setError(null);
			return;
		}

		// If it's already a valid URL, use it directly
		if (isValidUrl(assetPath)) {
			setUrl(assetPath);
			setIsLoading(false);
			setError(null);
			return;
		}

		// Handle local file paths - convert to asset:// for Tauri or use file://
		if (isLocalFilePath(assetPath)) {
			// In Tauri, local files should be accessed via the asset protocol
			// https://tauri.app/v1/api/js/tauri/#convertfilesrc
			const localUrl = `asset://localhost${assetPath}`;
			setUrl(localUrl);
			setIsLoading(false);
			setError(null);
			return;
		}

		// Need to resolve storage path to signed URL
		if (!appId || !backend?.storageState) {
			// Don't set error - just use path as-is and let the loader handle any 404
			// This allows components to work in preview mode or when context isn't ready
			setUrl(assetPath);
			setIsLoading(false);
			setError(null);
			return;
		}

		// Clean up path (remove storage:// prefix if present)
		const cleanPath = assetPath.replace(/^storage:\/\//, "");
		const cacheKey = `${appId}:${cleanPath}`;

		// Check cache first
		const cached = urlCache.get(cacheKey);
		if (isUsable(cached)) {
			setUrl(cached.url);
			setIsLoading(false);
			setError(null);
			return;
		}

		// Abort any previous request. The guard below has to read this run's own
		// controller: `abortControllerRef.current` is whatever the *newest* run
		// installed, which is never aborted, so checking it let a superseded
		// response overwrite the current one.
		abortControllerRef.current?.abort();
		const controller = new AbortController();
		abortControllerRef.current = controller;

		setIsLoading(true);
		setError(null);

		backend.storageState
			.downloadStorageItems(appId, [cleanPath])
			.then((results) => {
				if (controller.signal.aborted) return;

				const result = results[0];
				if (result?.url) {
					// Cache the URL
					urlCache.set(cacheKey, {
						url: result.url,
						expiresAt: Date.now() + CACHE_DURATION_MS,
					});
					setUrl(result.url);
					setError(null);
				} else if (result?.error) {
					// Fall back to raw path on error
					console.warn(
						"[useAssetUrl] Storage error, falling back to raw path:",
						result.error,
					);
					setUrl(cleanPath);
					setError(null);
				} else {
					// No result, fall back to raw path
					setUrl(cleanPath);
					setError(null);
				}
				setIsLoading(false);
			})
			.catch((err) => {
				if (controller.signal.aborted) return;
				console.warn(
					"[useAssetUrl] Failed to resolve asset URL, falling back to raw path:",
					err,
				);
				// Fall back to the raw path - let the loader handle the actual fetch
				setUrl(cleanPath);
				setError(null);
				setIsLoading(false);
			});

		return () => {
			abortControllerRef.current?.abort();
		};
	}, [assetPath, appId, backend?.storageState]);

	return { url, isLoading, error };
}

/**
 * Resolve multiple asset paths at once (batch request)
 */
export function useAssetUrls(assetPaths: string[]): {
	urls: Record<string, string>;
	isLoading: boolean;
	errors: Record<string, string>;
} {
	const backend = useBackend();
	const { appId } = useActionContext();
	const [urls, setUrls] = useState<Record<string, string>>({});
	const [isLoading, setIsLoading] = useState(false);
	const [errors, setErrors] = useState<Record<string, string>>({});

	useEffect(() => {
		if (!assetPaths.length) {
			setUrls({});
			setIsLoading(false);
			setErrors({});
			return;
		}

		const newUrls: Record<string, string> = {};
		const newErrors: Record<string, string> = {};
		const pathsToResolve: string[] = [];

		// Check which paths need resolution
		for (const path of assetPaths) {
			if (!path) continue;

			if (isValidUrl(path)) {
				newUrls[path] = path;
				continue;
			}

			const cleanPath = path.replace(/^storage:\/\//, "");
			const cacheKey = appId ? `${appId}:${cleanPath}` : cleanPath;

			const cached = urlCache.get(cacheKey);
			if (isUsable(cached)) {
				newUrls[path] = cached.url;
			} else {
				pathsToResolve.push(cleanPath);
			}
		}

		// If all are cached or direct URLs, return immediately
		if (!pathsToResolve.length) {
			setUrls(newUrls);
			setErrors(newErrors);
			setIsLoading(false);
			return;
		}

		if (!appId || !backend?.storageState) {
			for (const path of pathsToResolve) {
				newErrors[path] = "Cannot resolve storage path: missing app context";
			}
			setUrls(newUrls);
			setErrors(newErrors);
			setIsLoading(false);
			return;
		}

		setIsLoading(true);

		backend.storageState
			.downloadStorageItems(appId, pathsToResolve)
			.then((results) => {
				for (const result of results) {
					const originalPath = assetPaths.find(
						(p) => p.replace(/^storage:\/\//, "") === result.prefix,
					);
					if (!originalPath) continue;

					if (result.url) {
						const cacheKey = `${appId}:${result.prefix}`;
						urlCache.set(cacheKey, {
							url: result.url,
							expiresAt: Date.now() + CACHE_DURATION_MS,
						});
						newUrls[originalPath] = result.url;
					} else if (result.error) {
						newErrors[originalPath] = result.error;
					}
				}
				setUrls(newUrls);
				setErrors(newErrors);
				setIsLoading(false);
			})
			.catch((err) => {
				console.error("[useAssetUrls] Failed to resolve asset URLs:", err);
				for (const path of pathsToResolve) {
					newErrors[path] = err.message || "Failed to resolve asset URL";
				}
				setUrls(newUrls);
				setErrors(newErrors);
				setIsLoading(false);
			});
	}, [assetPaths.join(","), appId, backend?.storageState]);

	return { urls, isLoading, errors };
}
