"use client";

import { cn } from "../../../lib/utils";
import { Avatar, AvatarFallback, AvatarImage } from "../../ui/avatar";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { useAssetUrl } from "../hooks/use-asset-url";
import type { AvatarComponent, BoundValue } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

const sizeClasses: Record<string, string> = {
	sm: "h-8 w-8",
	md: "h-10 w-10",
	lg: "h-12 w-12",
	xl: "h-16 w-16",
};

export function A2UIAvatar({
	component,
	style,
}: ComponentProps<AvatarComponent>) {
	const { url: src } = useAssetUrl(useResolved<string>(component.src));
	const fallback = useResolved<string>(component.fallback);
	const size = useResolved<string>(component.size);

	const sizeClass = sizeClasses[size ?? "md"] ?? sizeClasses.md;

	return (
		<Avatar
			className={cn(sizeClass, resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{src && <AvatarImage src={src} />}
			<AvatarFallback>{fallback ?? "?"}</AvatarFallback>
		</Avatar>
	);
}
