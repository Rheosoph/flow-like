"use client";

import { DynamicIcon, type IconName, iconNames } from "lucide-react/dynamic";
import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, IconComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

const lucideIconNames = new Set<string>(iconNames);

function toKebabCase(str: string): string {
	return str
		.trim()
		.replace(/([a-z0-9])([A-Z])/g, "$1-$2")
		.replace(/([A-Z])([A-Z][a-z])/g, "$1-$2")
		.replace(/[_\s]+/g, "-")
		.toLowerCase();
}

export function A2UIIcon({ component, style }: ComponentProps<IconComponent>) {
	const iconName = useResolved<string>(component.name);
	const size = useResolved<string | number>(component.size);
	const color = useResolved<string>(component.color);
	const strokeWidth = useResolved<number>(component.strokeWidth);

	const resolvedIconName = iconName ? toKebabCase(iconName) : null;

	if (!resolvedIconName || !lucideIconNames.has(resolvedIconName)) {
		return (
			<span
				className={cn(
					"inline-flex items-center justify-center",
					resolveStyle(style),
				)}
				style={resolveInlineStyle(style)}
			>
				?
			</span>
		);
	}

	return (
		<DynamicIcon
			name={resolvedIconName as IconName}
			className={cn(resolveStyle(style))}
			style={{
				width: size ?? "1em",
				height: size ?? "1em",
				color,
				...resolveInlineStyle(style),
			}}
			strokeWidth={strokeWidth ?? 2}
		/>
	);
}
