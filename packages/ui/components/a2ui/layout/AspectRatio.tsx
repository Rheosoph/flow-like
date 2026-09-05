"use client";

import { Fragment } from "react";
import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { resolveChildSpecs } from "../children";
import type { AspectRatioComponent } from "../types";

export function A2UIAspectRatio({
	elementRef,
	component,
	style,
	renderChild,
}: ComponentProps<AspectRatioComponent>) {
	const { resolve } = useData();
	const children = resolveChildSpecs(component.children, resolve);
	const ratio = (resolve(component.ratio) as number) || 1;

	return (
		<div
			ref={elementRef}
			className={cn("relative w-full", resolveStyle(style))}
			style={{
				...resolveInlineStyle(style),
				aspectRatio: ratio,
			}}
		>
			<div className="absolute inset-0">
				{children.map((child) => (
					<Fragment key={child.key}>
						{renderChild(child.id, child.scope)}
					</Fragment>
				))}
			</div>
		</div>
	);
}
