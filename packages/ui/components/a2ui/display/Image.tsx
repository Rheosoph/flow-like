"use client";

import { useCallback } from "react";
import { useAssetImage } from "../../../hooks/use-asset-image";
import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { useAssetUrl } from "../hooks/use-asset-url";
import { useElementRef } from "../hooks/use-element-ref";
import type { BoundValue, ImageComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

const FIT_CLASSES: Record<string, string> = {
	contain: "object-contain",
	cover: "object-cover",
	fill: "object-fill",
	none: "object-none",
	scaleDown: "object-scale-down",
	"scale-down": "object-scale-down",
};

export function A2UIImage({
	elementRef,
	component,
	style,
}: ComponentProps<ImageComponent>) {
	const src = useResolved<string>(component.src);
	const alt = useResolved<string>(component.alt);
	const fit = useResolved<string>(component.fit);
	const rawFallback = useResolved<string>(component.fallback);
	const loading = useResolved<"lazy" | "eager">(component.loading);

	// The component holds a durable storage path; the signed URL that loads it
	// is minted here and renewed before it lapses.
	const { url: resolvedSrc, isLoading, refresh } = useAssetUrl(src);
	const { url: fallback } = useAssetUrl(rawFallback);

	// Sources here are usually signed storage URLs: they expire, and a dead one
	// has a live replacement the registry already knows about. Failure state is
	// keyed by URL, so pointing the component at another asset clears it.
	const image = useAssetImage(resolvedSrc);
	const rootRef = useElementRef(elementRef, image.imgRef);

	// A URL that failed may simply have outlived its signature: ask for a new
	// one before falling back. The request is rate-limited, so a link that is
	// dead for any other reason settles on the fallback after a single retry.
	const onError = useCallback(() => {
		image.onError();
		refresh();
	}, [image.onError, refresh]);

	const className = cn(fit && FIT_CLASSES[fit], resolveStyle(style));
	const inlineStyle = resolveInlineStyle(style);

	if (!image.canRender && fallback && !isLoading) {
		return (
			<img
				ref={elementRef}
				src={fallback}
				alt={alt ?? ""}
				className={className}
				style={inlineStyle}
			/>
		);
	}

	return (
		<img
			ref={rootRef}
			src={image.src}
			alt={alt ?? ""}
			loading={loading ?? "lazy"}
			onLoad={image.onLoad}
			onError={onError}
			className={className}
			style={inlineStyle}
		/>
	);
}
