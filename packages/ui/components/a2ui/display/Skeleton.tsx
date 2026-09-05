"use client";

import { cn } from "../../../lib/utils";
import { Skeleton } from "../../ui/skeleton";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, SkeletonComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UISkeleton({
	elementRef,
	component,
	style,
}: ComponentProps<SkeletonComponent>) {
	const width = useResolved<string | number>(component.width);
	const height = useResolved<string | number>(component.height);
	const rounded = useResolved<boolean>(component.rounded);

	return (
		<Skeleton
			ref={elementRef}
			className={cn(rounded && "rounded-full", resolveStyle(style))}
			style={{
				width,
				height,
				...resolveInlineStyle(style),
			}}
		/>
	);
}
