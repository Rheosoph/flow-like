"use client";

import { Fragment } from "react";
import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { resolveChildSpecs } from "../children";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { CenterComponent } from "../types";

export function A2UICenter({
	component,
	style,
	renderChild,
}: ComponentProps<CenterComponent>) {
	const { resolve } = useData();

	const inline = component.inline
		? (resolve(component.inline) as boolean)
		: false;
	const children = resolveChildSpecs(component.children, resolve);

	return (
		<div
			className={cn(
				inline ? "inline-flex" : "flex",
				"items-center justify-center",
				resolveStyle(style),
			)}
			style={resolveInlineStyle(style)}
		>
			{children.map((child) => (
				<Fragment key={child.key}>
					{renderChild(child.id, child.scope)}
				</Fragment>
			))}
		</div>
	);
}
