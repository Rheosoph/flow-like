"use client";

import { useState } from "react";
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

export function A2UIImage({
	component,
	style,
}: ComponentProps<ImageComponent>) {
	const src = useResolved<string>(component.src);
	const alt = useResolved<string>(component.alt);
	const fit = useResolved<string>(component.fit);
	const fallback = useResolved<string>(component.fallback);
	const loading = useResolved<"lazy" | "eager">(component.loading);
	const [error, setError] = useState(false);

	const fitMap: Record<string, string> = {
		contain: "object-contain",
		cover: "object-cover",
		fill: "object-fill",
		none: "object-none",
		scaleDown: "object-scale-down",
		"scale-down": "object-scale-down",
	};

	if (error && fallback) {
		return (
			<img
				src={fallback}
				alt={alt ?? ""}
				className={cn(fit && fitMap[fit], resolveStyle(style))}
				style={resolveInlineStyle(style)}
			/>
		);
	}

	return (
		<img
			src={src}
			alt={alt ?? ""}
			loading={loading ?? "lazy"}
			onError={() => setError(true)}
			className={cn(fit && fitMap[fit], resolveStyle(style))}
			style={resolveInlineStyle(style)}
		/>
	);
}
