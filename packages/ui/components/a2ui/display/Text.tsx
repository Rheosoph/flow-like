"use client";

import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, TextComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

const variantMap: Record<string, string> = {
	body: "text-base",
	heading: "font-bold",
	label: "text-sm font-medium",
	caption: "text-xs text-muted-foreground",
	code: "font-mono text-sm bg-muted px-1 py-0.5 rounded",
};

const sizeMap: Record<string, string> = {
	xs: "text-xs",
	sm: "text-sm",
	md: "text-base",
	lg: "text-lg",
	xl: "text-xl",
	"2xl": "text-2xl",
	"3xl": "text-3xl",
	"4xl": "text-4xl",
};

const weightMap: Record<string, string> = {
	light: "font-light",
	normal: "font-normal",
	medium: "font-medium",
	semibold: "font-semibold",
	bold: "font-bold",
};

const alignMap: Record<string, string> = {
	left: "text-left",
	center: "text-center",
	right: "text-right",
	justify: "text-justify",
};

export function A2UIText({
	elementRef,
	component,
	style,
}: ComponentProps<TextComponent>) {
	const content = useResolved<string>(component.content);
	const variant = useResolved<string>(component.variant);
	const size = useResolved<string>(component.size);
	const weight = useResolved<string>(component.weight);
	const align = useResolved<string>(component.align);
	const color = useResolved<string>(component.color);
	const truncate = useResolved<boolean>(component.truncate);
	const maxLines = useResolved<number>(component.maxLines);

	const Tag = variant === "heading" ? "h2" : "span";

	return (
		<Tag
			ref={elementRef}
			className={cn(
				variant && variantMap[variant],
				size && sizeMap[size],
				weight && weightMap[weight],
				align && alignMap[align],
				truncate && "truncate",
				maxLines && `line-clamp-${maxLines}`,
				resolveStyle(style),
			)}
			style={{
				color,
				...resolveInlineStyle(style),
			}}
		>
			{content}
		</Tag>
	);
}
