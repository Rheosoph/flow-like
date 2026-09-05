"use client";

import { Loader2 } from "lucide-react";
import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, SpinnerComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

const sizeMap: Record<string, string> = {
	sm: "h-4 w-4",
	md: "h-6 w-6",
	lg: "h-8 w-8",
};

export function A2UISpinner({
	elementRef,
	component,
	style,
}: ComponentProps<SpinnerComponent>) {
	const size = useResolved<string>(component.size);
	const sizeClass = sizeMap[size ?? "md"] ?? sizeMap.md;

	return (
		<Loader2
			ref={elementRef}
			className={cn("animate-spin", sizeClass, resolveStyle(style))}
			style={resolveInlineStyle(style)}
		/>
	);
}
