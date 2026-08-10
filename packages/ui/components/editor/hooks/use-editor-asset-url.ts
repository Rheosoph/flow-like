"use client";

import { useContext, useEffect, useRef, useState } from "react";
import { useBackend } from "../../../state/backend-state";
import { AIUsageAppContext } from "../ai-usage-context";
import { isStorageUrl, storagePathFromUrl } from "../upload-context";

interface CacheEntry {
	url: string;
	expiresAt: number;
}

const signedUrlCache = new Map<string, CacheEntry>();

/** Signed URLs are valid for 24h; refresh well before that so a long session never serves a dead one. */
const CACHE_DURATION_MS = 30 * 60 * 1000;

/**
 * Resolve a media reference for display.
 *
 * Editor documents persist `storage://…` paths rather than signed URLs, because a signed URL
 * expires long before the document does. Everything else (http, data, blob, asset) passes
 * through untouched.
 */
export function useEditorAssetUrl(url: string | undefined): string | undefined {
	const backend = useBackend();
	const appId = useContext(AIUsageAppContext);
	const [resolved, setResolved] = useState<string | undefined>(
		isStorageUrl(url) ? undefined : url,
	);
	const requestRef = useRef(0);

	useEffect(() => {
		if (!isStorageUrl(url)) {
			setResolved(url);
			return;
		}

		const path = storagePathFromUrl(url);
		const cacheKey = `${appId ?? "no-app"}:${path}`;
		const cached = signedUrlCache.get(cacheKey);
		if (cached && cached.expiresAt > Date.now()) {
			setResolved(cached.url);
			return;
		}

		if (!appId || !backend?.storageState) {
			setResolved(undefined);
			return;
		}

		const request = ++requestRef.current;
		let cancelled = false;

		backend.storageState
			.downloadStorageItems(appId, [path])
			.then((results) => {
				if (cancelled || request !== requestRef.current) return;
				const signed = results[0]?.url;
				if (!signed) {
					setResolved(undefined);
					return;
				}
				signedUrlCache.set(cacheKey, {
					url: signed,
					expiresAt: Date.now() + CACHE_DURATION_MS,
				});
				setResolved(signed);
			})
			.catch(() => {
				if (cancelled || request !== requestRef.current) return;
				setResolved(undefined);
			});

		return () => {
			cancelled = true;
		};
	}, [url, appId, backend?.storageState]);

	return resolved;
}
