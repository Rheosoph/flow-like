"use client";

import { Fragment } from "react";
import { cn } from "../../../lib/utils";
import type { ComponentProps, RenderChildFn } from "../ComponentRegistry";
import { resolveChildSpecs } from "../children";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { AbsoluteComponent, BoundValue } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UIAbsolute({
	component,
	style,
	renderChild,
}: ComponentProps<AbsoluteComponent> & { renderChild: RenderChildFn }) {
	const { resolve } = useData();
	const width = useResolved<string>(component.width);
	const height = useResolved<string>(component.height);
	const children = resolveChildSpecs(component.children, resolve);

	return (
		<div
			className={cn("relative", resolveStyle(style))}
			style={{
				width: width ?? "100%",
				height: height ?? "100%",
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
