"use client";

import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { SpacerComponent } from "../types";

export function A2UISpacer({
	elementRef,
	component,
	style,
}: ComponentProps<SpacerComponent>) {
	const { resolve } = useData();

	const size = component.size ? (resolve(component.size) as string) : undefined;
	const flex = component.flex
		? (resolve(component.flex) as number)
		: size
			? undefined
			: 1;

	return (
		<div
			ref={elementRef}
			className={cn(resolveStyle(style))}
			style={{
				...(size ? { width: size, height: size, flexShrink: 0 } : {}),
				...(flex !== undefined ? { flex: flex } : {}),
				...resolveInlineStyle(style),
			}}
			aria-hidden="true"
		/>
	);
}
