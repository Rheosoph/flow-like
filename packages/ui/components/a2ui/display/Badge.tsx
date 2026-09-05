"use client";

import { cn } from "../../../lib/utils";
import { Badge } from "../../ui/badge";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BadgeComponent, BoundValue } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

const variantMap: Record<
	string,
	"default" | "secondary" | "destructive" | "outline"
> = {
	default: "default",
	secondary: "secondary",
	destructive: "destructive",
	outline: "outline",
};

export function A2UIBadge({
	elementRef,
	component,
	style,
}: ComponentProps<BadgeComponent>) {
	const content = useResolved<string>(component.content);
	const variantValue = useResolved<string>(component.variant);
	const variant = variantMap[variantValue ?? "default"] ?? "default";

	return (
		<Badge
			ref={elementRef}
			variant={variant}
			className={cn(resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{content}
		</Badge>
	);
}
