"use client";

import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { useAssetUrl } from "../hooks/use-asset-url";
import type { BoundValue, SpriteComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UISprite({
	elementRef,
	component,
	style,
}: ComponentProps<SpriteComponent>) {
	const { url: src } = useAssetUrl(useResolved<string>(component.src));
	const x = useResolved<number>(component.x) ?? 0;
	const y = useResolved<number>(component.y) ?? 0;
	const width = useResolved<number>(component.width);
	const height = useResolved<number>(component.height);
	const rotation = useResolved<number>(component.rotation) ?? 0;
	const scale = useResolved<number>(component.scale) ?? 1;
	const opacity = useResolved<number>(component.opacity) ?? 1;
	const flipX = useResolved<boolean>(component.flipX);
	const flipY = useResolved<boolean>(component.flipY);
	const zIndex = useResolved<number>(component.zIndex);

	if (!src) return null;

	const transforms: string[] = [];
	if (rotation !== 0) transforms.push(`rotate(${rotation}deg)`);
	if (scale !== 1) transforms.push(`scale(${scale})`);
	if (flipX) transforms.push("scaleX(-1)");
	if (flipY) transforms.push("scaleY(-1)");

	return (
		<img
			ref={elementRef}
			src={src}
			alt=""
			className={cn("absolute pointer-events-auto", resolveStyle(style))}
			style={{
				left: x,
				top: y,
				width: width ?? "auto",
				height: height ?? "auto",
				opacity,
				transform: transforms.length > 0 ? transforms.join(" ") : undefined,
				zIndex,
				...resolveInlineStyle(style),
			}}
		/>
	);
}
