"use client";

import { Info, TriangleAlert } from "lucide-react";
import { type ReactNode, useId } from "react";
import { cn } from "../../../lib/utils";

/** One step's panel, with the heading rhythm of the look studio. */
export function BotStepPanel({
	title,
	subtitle,
	actions,
	children,
	className,
}: Readonly<{
	title: string;
	subtitle?: string;
	actions?: ReactNode;
	children: ReactNode;
	className?: string;
}>) {
	const headingId = useId();
	return (
		<section
			aria-labelledby={headingId}
			className={cn("flex min-w-0 flex-col gap-6", className)}
		>
			<header className="flex items-start justify-between gap-4">
				<div className="min-w-0">
					<h3
						id={headingId}
						className="text-lg font-semibold tracking-tight text-foreground"
					>
						{title}
					</h3>
					{subtitle && (
						<p className="mt-2 text-xs leading-relaxed text-muted-foreground">
							{subtitle}
						</p>
					)}
				</div>
				{actions}
			</header>
			{children}
		</section>
	);
}

export function BotSectionHeading({
	children,
	aside,
}: Readonly<{ children: ReactNode; aside?: ReactNode }>) {
	return (
		<div className="flex items-center justify-between gap-3">
			<h4 className="text-[0.6875rem] font-bold tracking-[0.12em] text-muted-foreground uppercase">
				{children}
			</h4>
			{aside}
		</div>
	);
}

export function BotNotice({
	tone = "info",
	live = tone === "warning",
	children,
	className,
}: Readonly<{
	tone?: "info" | "warning";
	/** Announced when it appears; warnings are by default. */
	live?: boolean;
	children: ReactNode;
	className?: string;
}>) {
	const Icon = tone === "warning" ? TriangleAlert : Info;
	return (
		<div
			role={live ? "status" : undefined}
			className={cn(
				"flex items-start gap-2.5 rounded-xl border px-3.5 py-3 text-xs leading-relaxed",
				tone === "warning"
					? "border-warning-line bg-warning-bg text-warning"
					: "border-info-line bg-info-bg text-info",
				className,
			)}
		>
			<Icon className="mt-px size-4 shrink-0" aria-hidden />
			<div className="min-w-0 flex-1">{children}</div>
		</div>
	);
}

/** A small rounded tag like the studio's series tag. */
export function BotTag({
	children,
	className,
}: Readonly<{ children: ReactNode; className?: string }>) {
	return (
		<span
			className={cn(
				"inline-flex items-center gap-1 rounded-md bg-muted px-2 py-1 text-[0.6875rem] font-medium text-muted-foreground",
				className,
			)}
		>
			{children}
		</span>
	);
}
