"use client";

import { Fragment } from "react";
import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { resolveChildSpecs } from "../children";
import { normalizeSemanticBoxTag } from "../semantic-box-tags";
import type { BoxComponent } from "../types";

export function A2UIBox({
	component,
	style,
	renderChild,
}: ComponentProps<BoxComponent>) {
	const { resolve } = useData();

	// `as` may come from generated JSON or a live data binding. Never pass an
	// arbitrary resolved string to React as an intrinsic element name.
	const Tag = normalizeSemanticBoxTag(
		component.as ? resolve(component.as) : undefined,
	);
	const children = resolveChildSpecs(component.children, resolve);

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
