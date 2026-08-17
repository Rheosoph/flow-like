"use client";

import { useTranslation } from "@flow-like/locales";
import { cn } from "../../../lib/utils";
import type { ComponentProps } from "../ComponentRegistry";
import { useData } from "../DataContext";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import type { BoundValue, HealthBarComponent } from "../types";

function useResolved<T>(boundValue: BoundValue | undefined): T | undefined {
	const { resolve } = useData();
	if (!boundValue) return undefined;
	return resolve(boundValue) as T;
}

export function A2UIHealthBar({
	component,
	style,
}: ComponentProps<HealthBarComponent>) {
	const { t } = useTranslation("common");
	const value = useResolved<number>(component.value) ?? 0;
	const maxValue = useResolved<number>(component.maxValue) ?? 100;
	const label = useResolved<string>(component.label);
	const variant = useResolved<string>(component.variant) ?? "bar";
	const fillColor = useResolved<string>(component.fillColor) ?? "#ef4444";
	const backgroundColor =
		useResolved<string>(component.backgroundColor) ?? "#374151";
	const showValue = useResolved<boolean>(component.showValue);

	const percentage = Math.min(100, Math.max(0, (value / maxValue) * 100));

	if (variant === "circular") {
		const radius = 40;
		const circumference = 2 * Math.PI * radius;
		const offset = circumference - (percentage / 100) * circumference;

		return (
			<div
				className={cn(
					"relative inline-flex items-center justify-center",
					resolveStyle(style),
				)}
				style={{ width: 100, height: 100, ...resolveInlineStyle(style) }}
			>
				<svg className="w-full h-full -rotate-90">
					<circle
						cx="50"
						cy="50"
						r={radius}
						fill="none"
						stroke={backgroundColor}
						strokeWidth="8"
					/>
					<circle
						cx="50"
						cy="50"
						r={radius}
						fill="none"
						stroke={fillColor}
						strokeWidth="8"
						strokeLinecap="round"
						strokeDasharray={circumference}
						strokeDashoffset={offset}
						className="transition-all duration-300"
					/>
				</svg>
				{showValue && (
					<span className="absolute text-sm font-bold">{`${value}/${maxValue}`}</span>
				)}
			</div>
		);
	}

	return (
		<div
			className={cn("w-full", resolveStyle(style))}
			style={resolveInlineStyle(style)}
		>
			{label && <div className="text-sm mb-1">{label}</div>}
			<div
				className="relative h-4 rounded-full overflow-hidden"
				style={{ backgroundColor }}
			>
				<div
					className="h-full rounded-full transition-all duration-300"
					style={{
						width: `${percentage}%`,
						backgroundColor: fillColor,
					}}
				/>
			</div>
			{showValue && (
				<div className="text-xs text-right mt-0.5">{`${value}/${maxValue}`}</div>
			)}
		</div>
	);
}
