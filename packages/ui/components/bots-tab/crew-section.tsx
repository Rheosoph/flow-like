"use client";

import { useTranslation } from "@flow-like/locales";
import { type ReactNode, useId } from "react";
import { cn } from "../../lib/utils";

/** A titled block on the Bots home stage. */
export function CrewSection({
	title,
	aside,
	actions,
	children,
}: Readonly<{
	title: ReactNode;
	aside?: ReactNode;
	actions?: ReactNode;
	children: ReactNode;
}>) {
	const headingId = useId();
	return (
		<section aria-labelledby={headingId} className="flex flex-col gap-3">
			<div className="flex flex-wrap items-center justify-between gap-3">
				<h2
					id={headingId}
					className="flex items-baseline gap-2 text-[15px] font-semibold"
				>
					{title}
					{aside}
				</h2>
				{actions && (
					<div className="flex flex-wrap items-center gap-2">{actions}</div>
				)}
			</div>
			{children}
		</section>
	);
}

export const CREW_CARD_CLASS =
	"rounded-2xl border bg-card text-left outline-none transition-[transform,border-color] duration-150 hover:border-foreground/20 focus-visible:ring-2 focus-visible:ring-ring motion-safe:hover:-translate-y-0.5";

export function crewCardClass(selected: boolean, className?: string) {
	return cn(CREW_CARD_CLASS, selected && "border-primary", className);
}

export function UnreadBadge({
	count,
	className,
}: Readonly<{ count: number; className?: string }>) {
	const { t } = useTranslation("bots");
	if (count <= 0) return null;
	const label = t("crew.unread", "{{count}} unread", { count });
	return (
		<span
			title={label}
			className={cn(
				"inline-flex h-5 min-w-5 shrink-0 items-center justify-center rounded-full bg-primary px-1.5 text-[11px] font-bold text-primary-foreground",
				className,
			)}
		>
			<span aria-hidden>{count > 99 ? "99+" : count}</span>
			<span className="sr-only">{label}</span>
		</span>
	);
}

export function StatusDot({
	pulse = false,
	muted = false,
}: Readonly<{ pulse?: boolean; muted?: boolean }>) {
	return (
		<span
			aria-hidden
			className={cn(
				"size-1.75 shrink-0 rounded-full",
				muted ? "bg-muted-foreground/60" : "bg-primary",
				pulse && "motion-safe:animate-pulse",
			)}
		/>
	);
}
