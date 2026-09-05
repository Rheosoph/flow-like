"use client";

import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { useAssetUrl } from "../hooks/use-asset-url";
import type { BoundValue, CharacterPortraitComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

const sizeClasses = {
	small: "w-16 h-16",
	medium: "w-24 h-24",
	large: "w-32 h-32",
};

const positionClasses = {
	left: "self-start",
	center: "self-center",
	right: "self-end",
};

export function A2UICharacterPortrait({
	elementRef,
	component,
	style,
}: ComponentProps<CharacterPortraitComponent>) {
	const { url: image } = useAssetUrl(useResolved<string>(component.image));
	const expression = useResolved<string>(component.expression);
	const size = useResolved<string>(component.size) ?? "medium";
	const position = useResolved<string>(component.position) ?? "center";
	const dimmed = useResolved<boolean>(component.dimmed) ?? false;

	const imageSrc = expression ? `${image}?expression=${expression}` : image;

	if (!image) return null;

	return (
		<div
			ref={elementRef}
			className={cn(
				"relative rounded-full overflow-hidden",
				sizeClasses[size as keyof typeof sizeClasses],
				positionClasses[position as keyof typeof positionClasses],
				dimmed && "opacity-50",
				resolveStyle(style),
			)}
			style={resolveInlineStyle(style)}
		>
			<img src={imageSrc} alt="" className="w-full h-full object-cover" />
		</div>
	);
}
