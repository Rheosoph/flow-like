"use client";

import { useCallback, useLayoutEffect, useRef, useState } from "react";
import {
	confirmStableAssetUrl,
	isExpiredAssetUrl,
	recoverStableAssetUrl,
} from "../lib/stable-asset-url";

export interface AssetImageState {
	/** URL to hand the element, or `undefined` when there is nothing to show. */
	src: string | undefined;
	/** False when the URL is dead or has failed — paint the fallback instead. */
	canRender: boolean;
	/** True once the bytes are decoded. Drive the fade-in from this. */
	loaded: boolean;
	imgRef: (node: HTMLImageElement | null) => void;
	onLoad: () => void;
	onError: () => void;
}

/**
 * Drives one `<img>` whose URL is a signed, expiring link.
 *
 * Three things go wrong with artwork behind signed URLs, and a component that
 * only tracks `onError` handles none of them:
 *
 * - A signature that has already expired cannot load, so asking for it costs a
 *   request and shows a broken image. Those URLs are skipped outright.
 * - A signature can expire while the page is open. The registry usually knows a
 *   newer one for the same object, so a failure retries that before giving up,
 *   and it gives up only for the exact URL that failed — a later refresh
 *   recovers on its own.
 * - An image appearing at full opacity the instant it decodes reads as a pop.
 *   `loaded` lets the caller cross-fade from whatever placeholder it draws.
 *
 * A cached image can finish decoding before React attaches `onLoad`, which
 * would strand a fade at zero opacity forever. Reading `complete` in a layout
 * effect catches that before the browser paints, so cached artwork appears at
 * once with no transition at all.
 */
export function useAssetImage(
	source: string | null | undefined,
): AssetImageState {
	const [failedSrc, setFailedSrc] = useState<string | null>(null);
	const [retry, setRetry] = useState<{ source: string; url: string } | null>(
		null,
	);
	const [loadedSrc, setLoadedSrc] = useState<string | null>(null);
	const node = useRef<HTMLImageElement | null>(null);

	const src =
		(source && retry?.source === source ? retry.url : source) ?? undefined;

	const imgRef = useCallback((element: HTMLImageElement | null) => {
		node.current = element;
	}, []);

	useLayoutEffect(() => {
		const element = node.current;
		if (src && element?.complete && element.naturalWidth > 0) {
			setLoadedSrc(src);
		}
	}, [src]);

	const onLoad = useCallback(() => {
		if (!src) return;
		confirmStableAssetUrl(src);
		setLoadedSrc(src);
	}, [src]);

	const onError = useCallback(() => {
		if (!src) return;
		const recovered = recoverStableAssetUrl(src);
		if (recovered && recovered !== src) {
			setFailedSrc(null);
			setRetry({ source: source ?? src, url: recovered });
			return;
		}
		setFailedSrc(src);
	}, [source, src]);

	return {
		src,
		canRender:
			Boolean(src) && failedSrc !== src && !isExpiredAssetUrl(src ?? null),
		loaded: loadedSrc === src,
		imgRef,
		onLoad,
		onError,
	};
}
