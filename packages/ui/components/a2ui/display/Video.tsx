"use client";

import { useTranslation } from "@flow-like/locales";
import { useState } from "react";
import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, VideoComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UIVideo({
	component,
	style,
}: ComponentProps<VideoComponent>) {
	const { t } = useTranslation("common");
	const src = useResolved<string>(component.src);
	const poster = useResolved<string>(component.poster);
	const controls = useResolved<boolean>(component.controls);
	const autoplay = useResolved<boolean>(component.autoplay);
	const loop = useResolved<boolean>(component.loop);
	const muted = useResolved<boolean>(component.muted);
	const [error, setError] = useState(false);

	if (error || !src) {
		return (
			<div
				className={cn(
					"flex items-center justify-center bg-muted text-muted-foreground",
					resolveStyle(style),
				)}
				style={resolveInlineStyle(style)}
			>
				{t('videoUnavailable', 'Video unavailable')}
			</div>
		);
	}

	return (
		<video
			src={src}
			poster={poster}
			controls={controls ?? true}
			autoPlay={autoplay ?? false}
			loop={loop ?? false}
			muted={muted ?? false}
			className={cn("w-full", resolveStyle(style))}
			style={resolveInlineStyle(style)}
			onError={() => setError(true)}
		/>
	);
}
