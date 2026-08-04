"use client";

import { Fragment } from "react";
import { cn } from "../../../lib/utils";
import { ScrollArea } from "../../ui/scroll-area";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { resolveChildSpecs } from "../children";
import type { BoundValue, ScrollAreaComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UIScrollArea({
	component,
	style,
	renderChild,
}: ComponentProps<ScrollAreaComponent>) {
	const { resolve } = useData();
	const children = resolveChildSpecs(component.children, resolve);
	const direction = useResolved<string>(component.direction);

	const viewportClass =
		direction === "horizontal"
			? "overflow-x-auto overflow-y-hidden"
			: direction === "both"
				? "overflow-auto"
				: "overflow-y-auto overflow-x-hidden";

	return (
		<ScrollArea
			className={cn("h-full w-full", resolveStyle(style))}
			viewportClassName={viewportClass}
			orientation={
				direction === "horizontal" || direction === "both"
					? direction
					: "vertical"
			}
			style={resolveInlineStyle(style)}
		>
			{children.map((child) => (
				<Fragment key={child.key}>
					{renderChild(child.id, child.scope)}
				</Fragment>
			))}
		</ScrollArea>
	);
}
