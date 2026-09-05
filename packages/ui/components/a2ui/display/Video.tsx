"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useState } from "react";
import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { useAssetUrl } from "../hooks/use-asset-url";
import type { BoundValue, VideoComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UIVideo({
	elementRef,
	component,
	style,
}: ComponentProps<VideoComponent>) {
	const { t } = useTranslation("common");
	const rawSrc = useResolved<string>(component.src);
	const rawPoster = useResolved<string>(component.poster);
	const { url: src, isLoading, refresh } = useAssetUrl(rawSrc);
	const { url: poster } = useAssetUrl(rawPoster);
	const controls = useResolved<boolean>(component.controls);
	const autoplay = useResolved<boolean>(component.autoplay);
	const loop = useResolved<boolean>(component.loop);
	const muted = useResolved<boolean>(component.muted);
	// Failure is keyed by URL, not a latch: a source that lapsed while the page
	// sat open gets a new signature, and pointing the element at it must clear
	// the verdict the dead one earned.
	const [failedSrc, setFailedSrc] = useState<string | null>(null);
	const error = Boolean(src) && failedSrc === src;

	const onError = useCallback(() => {
		if (src) setFailedSrc(src);
		refresh();
	}, [src, refresh]);

	if (error || (!src && !isLoading)) {
		return (
			<div
				ref={elementRef}
				className={cn(
					"flex items-center justify-center bg-muted text-muted-foreground",
					resolveStyle(style),
				)}
				style={resolveInlineStyle(style)}
			>
				{t("videoUnavailable", "Video unavailable")}
			</div>
		);
	}

	return (
		<video
			ref={elementRef}
			src={src}
			poster={poster}
			controls={controls ?? true}
			autoPlay={autoplay ?? false}
			loop={loop ?? false}
			muted={muted ?? false}
			className={cn("w-full", resolveStyle(style))}
			style={resolveInlineStyle(style)}
			onError={onError}
		/>
	);
}
