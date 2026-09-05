"use client";

import { cn } from "../../../lib/utils";
import { Progress } from "../../ui/progress";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, ProgressComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UIProgress({
	elementRef,
	component,
	style,
}: ComponentProps<ProgressComponent>) {
	const value = useResolved<number>(component.value);
	const resolvedMax = useResolved<number>(component.max);
	const variant = useResolved<string>(component.variant);
	const showLabel = useResolved<boolean>(component.showLabel);
	const max = resolvedMax ?? 100;

	const percent = Math.min(100, Math.max(0, ((value ?? 0) / max) * 100));

	const variantClasses: Record<string, string> = {
		default: "",
		success: "[&>div]:bg-green-500",
		warning: "[&>div]:bg-yellow-500",
		error: "[&>div]:bg-destructive",
	};

	return (
		<div
			ref={elementRef}
			className={cn("space-y-1", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			<Progress
				value={percent}
				className={cn(variantClasses[variant ?? "default"])}
			/>
			{showLabel && (
				<span className="text-xs text-muted-foreground">
					{Math.round(percent)}%
				</span>
			)}
		</div>
	);
}
