"use client";

import { useCallback, useEffect, useState } from "react";
import {
	type ResolvedAssetUrl,
	invalidateAssetUrl,
	isRootedPath,
	isStorageAssetPath,
	localFileAssetUrl,
	normalizeStorageAssetPath,
	peekAssetUrl,
	resolveAssetUrl,
} from "../lib/asset-url-cache";
import { useBackend, useBackendReady } from "../state/backend-state";

/** Never re-resolve faster than this, whatever the cache reports. */
const MIN_REFRESH_DELAY_MS = 1000;

export interface AssetSourceOptions {
	/**
	 * Read a rooted path as a file on disk and address it through the desktop
	 * shell's asset protocol, rather than leaving it to resolve against the
	 * page's own origin.
	 *
	 * Off by default: `/images/logo.png` on a page means the host's own asset far
	 * more often than it means `/images` on the user's disk. Only sources that
	 * genuinely carry filesystem paths — a 3D model handed over by a local
	 * workflow — should turn it on.
	 */
	readonly localFiles?: boolean;
}

export interface AssetSourceState {
	/** URL to hand the element, or `undefined` while the path is being signed. */
	readonly src: string | undefined;
	/** True only while a storage path has no URL to show yet. */
	readonly isLoading: boolean;
	/**
	 * Sign the path again now, for a caller that has just watched the current
	 * URL fail. A no-op for values that are not storage paths, and rate-limited
	 * so a link the store rejects for good cannot spin.
	 */
	readonly refresh: () => void;
}

/**
 * Turns whatever a component holds in its `src` — a storage path, a `data:`
 * URL, an ordinary link — into something an element can load, and keeps it
 * loadable.
 *
 * Only storage paths involve any work. Those are signed through
 * {@link resolveAssetUrl}, which batches concurrent requests and reuses a
 * signature until it is close to lapsing; this hook then re-resolves just
 * before that point, so a surface left open for a day keeps showing its
 * artwork instead of dying with the first credential that signed it.
 */
export function useAssetSource(
	appId: string | undefined,
	rawSrc: string | undefined,
	options: AssetSourceOptions = {},
): AssetSourceState {
	const { localFiles = false } = options;
	const backend = useBackend();
	const backendReady = useBackendReady();
	const storageState = backend.storageState;

	const storagePath = isStorageAssetPath(rawSrc)
		? normalizeStorageAssetPath(rawSrc)
		: undefined;
	const directSrc =
		!storagePath && rawSrc && localFiles && isRootedPath(rawSrc)
			? localFileAssetUrl(rawSrc)
			: storagePath
				? undefined
				: rawSrc;

	const [settled, setSettled] = useState<{
		path: string;
		entry: ResolvedAssetUrl;
	} | null>(null);
	const [attempt, setAttempt] = useState(0);

	// A path that changed has not settled yet, so fall back to whatever another
	// surface already resolved for it rather than blanking the element.
	const entry =
		settled && settled.path === storagePath
			? settled.entry
			: peekAssetUrl(appId, storagePath);

	const canResolve = Boolean(storagePath && appId && backendReady);

	// biome-ignore lint/correctness/useExhaustiveDependencies: `attempt` is the re-resolve trigger — the timer below and refresh() bump it to ask for a new signature.
	useEffect(() => {
		if (!canResolve || !storagePath || !appId) return;

		let cancelled = false;
		let timer: ReturnType<typeof setTimeout> | undefined;

		void resolveAssetUrl(appId, storagePath, storageState).then((resolved) => {
			if (cancelled) return;
			setSettled({ path: storagePath, entry: resolved });

			const delay = resolved.usableUntil - Date.now();
			if (!Number.isFinite(delay)) return;
			timer = setTimeout(
				() => setAttempt((previous) => previous + 1),
				Math.max(delay, MIN_REFRESH_DELAY_MS),
			);
		});

		return () => {
			cancelled = true;
			if (timer !== undefined) clearTimeout(timer);
		};
	}, [canResolve, appId, storagePath, storageState, attempt]);

	const refresh = useCallback(() => {
		if (!appId || !storagePath) return;
		if (!invalidateAssetUrl(appId, storagePath)) return;
		setAttempt((previous) => previous + 1);
	}, [appId, storagePath]);

	if (!storagePath) {
		return { src: directSrc, isLoading: false, refresh };
	}

	// No app to sign against — a builder preview, or a surface rendered before
	// the backend is up. Hand back the path and let the element's own error
	// handling deal with it, as it did before anything was signed at all.
	if (!canResolve) {
		return { src: storagePath, isLoading: false, refresh };
	}

	return { src: entry?.url, isLoading: !entry, refresh };
}
