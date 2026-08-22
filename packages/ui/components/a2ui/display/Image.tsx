"use client";

import { useAssetImage } from "../../../hooks/use-asset-image";
import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
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
	component,
	style,
}: ComponentProps<ImageComponent>) {
	const src = useResolved<string>(component.src);
	const alt = useResolved<string>(component.alt);
	const fit = useResolved<string>(component.fit);
	const fallback = useResolved<string>(component.fallback);
	const loading = useResolved<"lazy" | "eager">(component.loading);
	// Sources here are usually signed storage URLs: they expire, and a dead one
	// has a live replacement the registry already knows about. Failure state is
	// keyed by URL, so pointing the component at another asset clears it.
	const image = useAssetImage(src);

	const className = cn(fit && FIT_CLASSES[fit], resolveStyle(style));
	const inlineStyle = resolveInlineStyle(style);

	if (!image.canRender && fallback) {
		return (
			<img
				src={fallback}
				alt={alt ?? ""}
				className={className}
				style={inlineStyle}
			/>
		);
	}

	return (
		<img
			ref={image.imgRef}
			src={image.src}
			alt={alt ?? ""}
			loading={loading ?? "lazy"}
			onLoad={image.onLoad}
			onError={image.onError}
			className={className}
			style={inlineStyle}
		/>
	);
}
