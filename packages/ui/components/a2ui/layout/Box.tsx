"use client";

import type { CSSProperties, ElementType, ReactNode } from "react";
import { Fragment } from "react";
import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { resolveChildSpecs } from "../children";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoxComponent } from "../types";

export function A2UIBox({
	component,
	style,
	renderChild,
}: ComponentProps<BoxComponent>) {
	const { resolve } = useData();

	const as = component.as ? (resolve(component.as) as string) : "div";
	const children = resolveChildSpecs(component.children, resolve);

	const Tag = as as ElementType<{
		className?: string;
		style?: CSSProperties;
		children?: ReactNode;
	}>;

	return (
		<Tag className={cn(resolveStyle(style))} style={resolveInlineStyle(style)}>
			{children.map((child) => (
				<Fragment key={child.key}>
					{renderChild(child.id, child.scope)}
				</Fragment>
			))}
		</Tag>
	);
}
