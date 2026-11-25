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

const toneClasses: Record<string, string> = {
	neutral: "border-unknown-line bg-unknown-bg text-unknown",
	info: "border-info-line bg-info-bg text-info",
	success: "border-good-line bg-good-bg text-good",
	warning: "border-warning-line bg-warning-bg text-warning",
	danger: "border-critical-line bg-critical-bg text-critical",
};

export function A2UIBadge({
	elementRef,
	component,
	style,
}: ComponentProps<BadgeComponent>) {
	const content = useResolved<string>(component.content);
	const variantValue = useResolved<string>(component.variant);
	const variant = variantMap[variantValue ?? "default"] ?? "default";
	const tone = useResolved<string>(component.tone);
	const toneClass = tone ? toneClasses[tone] : undefined;

	return (
		<Badge
			ref={elementRef}
			variant={toneClass ? "outline" : variant}
			data-tone={toneClass ? tone : undefined}
			className={cn(toneClass, resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{content}
		</Badge>
	);
}
