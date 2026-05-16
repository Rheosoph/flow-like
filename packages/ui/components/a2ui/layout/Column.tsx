"use client";

import { Fragment } from "react";
import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { resolveChildSpecs } from "../children";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, ColumnComponent } from "../types";

function useResolved<T>(
	resolve: (bv: BoundValue) => unknown,
	boundValue: BoundValue | undefined,
): T | undefined {
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UIColumn({
	component,
	style,
	renderChild,
}: ComponentProps<ColumnComponent>) {
	const { resolve } = useData();

	const alignMap: Record<string, string> = {
		start: "items-start",
		center: "items-center",
		end: "items-end",
		stretch: "items-stretch",
		baseline: "items-baseline",
	};

	const justifyMap: Record<string, string> = {
		start: "justify-start",
		center: "justify-center",
		end: "justify-end",
		between: "justify-between",
		around: "justify-around",
		evenly: "justify-evenly",
	};

	const gap = useResolved<string>(resolve, component.gap);
	const align = useResolved<string>(resolve, component.align);
	const justify = useResolved<string>(resolve, component.justify);
	const wrap = useResolved<boolean>(resolve, component.wrap);
	const reverse = useResolved<boolean>(resolve, component.reverse);

	const children = resolveChildSpecs(component.children, resolve);

	return (
		<div
			className={cn(
				"flex flex-col",
				align && alignMap[align],
				justify && justifyMap[justify],
				reverse && "flex-col-reverse",
				wrap && "flex-wrap",
				resolveStyle(style),
			)}
			style={{
				gap,
				...resolveInlineStyle(style),
			}}
		>
			{children.map((child) => (
				<Fragment key={child.key}>
					{renderChild(child.id, child.scope)}
				</Fragment>
			))}
		</div>
	);
}
