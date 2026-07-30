"use client";

import {
	EyeIcon,
	GlobeIcon,
	LockIcon,
	type LucideIcon,
	WifiOffIcon,
} from "lucide-react";
import type { ReactNode } from "react";
import { IAppVisibility } from "../../../lib";
import { cn } from "../../../lib/utils";
import { Badge } from "../../ui/badge";
import { Card, CardContent } from "../../ui/card";

export function SectionCard({
	title,
	icon: Icon,
	count,
	action,
	children,
	className,
	contentClassName,
}: Readonly<{
	title: string;
	icon?: LucideIcon;
	count?: number;
	action?: ReactNode;
	children: ReactNode;
	className?: string;
	contentClassName?: string;
}>) {
	return (
		<Card className={cn("gap-0 overflow-hidden py-0", className)}>
			<div className="flex items-center gap-2 border-b px-4 py-2.5">
				{Icon && <Icon className="h-4 w-4 text-muted-foreground" />}
				<h3 className="text-sm font-semibold">{title}</h3>
				{count !== undefined && (
					<Badge variant="secondary" className="text-xs">
						{count}
					</Badge>
				)}
				{action && <div className="ml-auto flex items-center">{action}</div>}
			</div>
			<CardContent className={cn("p-4", contentClassName)}>
				{children}
			</CardContent>
		</Card>
	);
}

export function StateDot({
	tone = "ok",
	className,
}: Readonly<{
	tone?: "ok" | "warn" | "critical" | "idle";
	className?: string;
}>) {
	const toneClass = {
		ok: "bg-emerald-500",
		warn: "bg-amber-500",
		critical: "bg-destructive",
		idle: "bg-muted-foreground/50",
	}[tone];
	return (
		<span
			className={cn("h-2 w-2 shrink-0 rounded-full", toneClass, className)}
			aria-hidden
		/>
	);
}

export function Meter({
	value,
	total,
	tone = "primary",
	className,
}: Readonly<{
	value: number;
	total: number;
	tone?: "primary" | "ok" | "warn";
	className?: string;
}>) {
	const pct = total > 0 ? Math.min(100, Math.max(0, (value / total) * 100)) : 0;
	const toneClass = {
		primary: "bg-primary",
		ok: "bg-emerald-500",
		warn: "bg-amber-500",
	}[tone];
	// Decorative: every call site pairs the bar with a visible "x of y" label,
	// so announcing it again would only add noise for screen-reader users.
	return (
		<div
			className={cn(
				"h-1.5 w-full overflow-hidden rounded-full bg-muted",
				className,
			)}
			aria-hidden
		>
			<div
				className={cn("h-full rounded-full transition-all", toneClass)}
				style={{ width: `${pct}%` }}
			/>
		</div>
	);
}

/**
 * Tiny run-volume sparkline. Rendered from real bucket counts — when every
 * bucket is zero the caller should not mount it at all.
 */
export function Sparkline({
	values,
	className,
	tone = "ok",
}: Readonly<{ values: number[]; className?: string; tone?: "ok" | "warn" }>) {
	if (values.length < 2) return null;
	const max = Math.max(...values, 1);
	const step = 100 / (values.length - 1);
	const points = values.map((value, index) => {
		const x = index * step;
		const y = 26 - (value / max) * 22 - 2;
		return `${x.toFixed(2)} ${y.toFixed(2)}`;
	});
	const line = `M${points.join(" L")}`;
	const area = `${line} L100 26 L0 26 Z`;
	const stroke = tone === "warn" ? "var(--chart-4)" : "oklch(0.72 0.16 150)";

	return (
		<svg
			className={cn("h-7 w-full", className)}
			viewBox="0 0 100 26"
			preserveAspectRatio="none"
			aria-hidden
		>
			<title>Run volume, last 24 hours</title>
			<path d={area} fill={stroke} fillOpacity={0.14} />
			<path
				d={line}
				fill="none"
				stroke={stroke}
				strokeWidth={1.5}
				vectorEffect="non-scaling-stroke"
				strokeLinejoin="round"
				strokeLinecap="round"
			/>
		</svg>
	);
}

const VISIBILITY_CONFIG: Record<
	IAppVisibility,
	{ label: string; icon: LucideIcon }
> = {
	[IAppVisibility.Offline]: { label: "Offline", icon: WifiOffIcon },
	[IAppVisibility.Private]: { label: "Private", icon: LockIcon },
	[IAppVisibility.Prototype]: { label: "Prototype", icon: EyeIcon },
	[IAppVisibility.Public]: { label: "Public", icon: GlobeIcon },
	[IAppVisibility.PublicRequestAccess]: {
		label: "Request Access",
		icon: LockIcon,
	},
};

export function VisibilityBadge({
	visibility,
	className,
}: Readonly<{ visibility: IAppVisibility; className?: string }>) {
	const config = VISIBILITY_CONFIG[visibility] ?? VISIBILITY_CONFIG.Private;
	const Icon = config.icon;
	return (
		<Badge
			variant={visibility === IAppVisibility.Public ? "default" : "outline"}
			className={cn("gap-1 text-xs", className)}
		>
			<Icon className="h-3 w-3" />
			{config.label}
		</Badge>
	);
}

export function EmptyHint({
	children,
	className,
}: Readonly<{ children: ReactNode; className?: string }>) {
	return (
		<p
			className={cn(
				"py-4 text-center text-sm text-muted-foreground",
				className,
			)}
		>
			{children}
		</p>
	);
}
