"use client";

import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, DividerComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

const orientationClasses: Record<string, string> = {
	horizontal: "w-full h-px",
	vertical: "h-full w-px",
};

export function A2UIDivider({
	elementRef,
	component,
	style,
}: ComponentProps<DividerComponent>) {
	const orientationValue = useResolved<string>(component.orientation);
	const color = useResolved<string>(component.color);
	const thickness = useResolved<string | number>(component.thickness);
	const orientation = orientationValue ?? "horizontal";

	return (
		<div
			ref={elementRef}
			className={cn(
				"bg-border shrink-0",
				orientationClasses[orientation],
				resolveStyle(style),
			)}
			style={{
				backgroundColor: color,
				...(orientation === "horizontal"
					? { height: thickness }
					: { width: thickness }),
				...resolveInlineStyle(style),
			}}
			role="separator"
			aria-orientation={orientation as "horizontal" | "vertical"}
		/>
	);
}
