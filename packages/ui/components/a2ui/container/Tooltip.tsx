"use client";

import { Fragment } from "react";
import { cn } from "../../../lib/utils";
import {
	Tooltip as ShadTooltip,
	TooltipContent,
	TooltipProvider,
	TooltipTrigger,
} from "../../ui/tooltip";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, Children, TooltipComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

function getChildIds(children: Children | undefined): string[] {
	if (!children) return [];
	if ("explicitList" in children) return children.explicitList;
	return [];
}

const sideMap: Record<string, "top" | "bottom" | "left" | "right"> = {
	top: "top",
	bottom: "bottom",
	left: "left",
	right: "right",
};

export function A2UITooltip({
	elementRef,
	component,
	style,
	renderChild,
}: ComponentProps<TooltipComponent>) {
	const content = useResolved<string>(component.content);
	const side = useResolved<string>(component.side);
	const delayMs = useResolved<number>(component.delayMs);
	const childIds = getChildIds(component.children);
	const resolvedSide = sideMap[side ?? "top"] ?? "top";

	return (
		<TooltipProvider>
			<ShadTooltip delayDuration={delayMs ?? 200}>
				<TooltipTrigger asChild>
					<span
						ref={elementRef}
						className={cn("inline-block", resolveStyle(style))}
						style={resolveInlineStyle(style)}
					>
						{childIds.map((id) => (
							<Fragment key={id}>{renderChild(id)}</Fragment>
						))}
					</span>
				</TooltipTrigger>
				<TooltipContent side={resolvedSide}>{content}</TooltipContent>
			</ShadTooltip>
		</TooltipProvider>
	);
}
