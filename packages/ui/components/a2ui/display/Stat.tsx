"use client";

import { ArrowDownRight, ArrowRight, ArrowUpRight } from "lucide-react";
import { DynamicIcon, type IconName, iconNames } from "lucide-react/dynamic";
import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, StatComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

const lucideIconNames = new Set<string>(iconNames);

const trendStyles = {
	up: { Arrow: ArrowUpRight, ink: "text-good" },
	down: { Arrow: ArrowDownRight, ink: "text-critical" },
	flat: { Arrow: ArrowRight, ink: "text-muted-foreground" },
} as const;

function iconSlug(name: string): string {
	return name
		.trim()
		.replace(/([a-z0-9])([A-Z])/g, "$1-$2")
		.replace(/[_\s]+/g, "-")
		.toLowerCase();
}

function text(value: unknown): string | undefined {
	if (value === undefined || value === null || value === "") return undefined;
	return String(value);
}

export function A2UIStat({
	elementRef,
	component,
	style,
}: ComponentProps<StatComponent>) {
	const label = text(useResolved<unknown>(component.label));
	const value = text(useResolved<unknown>(component.value));
	const change = text(useResolved<unknown>(component.change));
	const trendValue = useResolved<string>(component.trend);
	const caption = text(useResolved<unknown>(component.caption));
	const iconValue = useResolved<string>(component.icon);
	const slug = iconValue ? iconSlug(iconValue) : undefined;
	const trend =
		trendValue && trendValue in trendStyles
			? trendStyles[trendValue as keyof typeof trendStyles]
			: undefined;

	return (
		<div
			ref={elementRef}
			data-slot="stat"
			className={cn(
				"flex min-w-0 flex-col gap-1 rounded-lg bg-muted/60 p-3",
				resolveStyle(style),
			)}
			style={resolveInlineStyle(style)}
		>
			<div className="flex items-center gap-1.5 text-xs text-muted-foreground">
				{slug && lucideIconNames.has(slug) && (
					<DynamicIcon name={slug as IconName} className="size-3.5 shrink-0" />
				)}
				<span className="truncate">{label}</span>
			</div>
			<div className="text-2xl font-medium tabular-nums">{value}</div>
			{change && (
				<div
					data-slot="stat-change"
					data-trend={trendValue}
					className={cn(
						"flex items-center gap-1 text-xs font-medium tabular-nums",
						trend?.ink ?? "text-muted-foreground",
					)}
				>
					{trend && <trend.Arrow className="size-3.5 shrink-0" />}
					<span>{change}</span>
				</div>
			)}
			{caption && (
				<div className="text-xs text-muted-foreground">{caption}</div>
			)}
		</div>
	);
}
