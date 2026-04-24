"use client";

import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { resolveChildSpecs } from "../children";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, StackComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UIStack({
	component,
	style,
	renderChild,
}: ComponentProps<StackComponent>) {
	const { resolve } = useData();

	const alignMap: Record<string, string> = {
		start: "items-start",
		center: "items-center",
		end: "items-end",
		stretch: "items-stretch",
	};

	const align = useResolved<string>(component.align);
	const width = useResolved<string>(component.width);
	const height = useResolved<string>(component.height);

	const children = resolveChildSpecs(component.children, resolve);

	// Build inline styles from component props
	const inlineStyles = {
		...resolveInlineStyle(style),
		...(width && { width }),
		...(height && { height }),
	};

	return (
		<div
			className={cn(
				"relative",
				align && alignMap[align],
				// Ensure stack has minimum dimensions when empty
				children.length === 0 && "min-h-[100px] min-w-[100px]",
				resolveStyle(style),
			)}
			style={inlineStyles}
		>
			{children.map((child, index) => (
				<div
					key={child.key}
					className="absolute inset-0"
					style={{ zIndex: index }}
				>
					{renderChild(child.id, child.scope)}
				</div>
			))}
		</div>
	);
}
