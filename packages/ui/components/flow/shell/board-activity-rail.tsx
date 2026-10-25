"use client";

import Link from "next/link";
import type { ReactNode } from "react";
import { memo } from "react";
import { cn } from "../../../lib/utils";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";

export interface IBoardRailItem {
	id: string;
	title: string;
	icon: ReactNode;
	active?: boolean;
	/** Rendered as a counter dot; `0` and `undefined` render nothing. */
	badge?: number;
	/** Describes the full count and what it measures in the tooltip and accessible name. */
	badgeLabel?: string;
	badgeTone?: "default" | "warning" | "danger";
	shortcut?: string;
	href?: string;
	onSelect: () => void;
}

const BADGE_TONE: Record<string, string> = {
	default: "bg-primary text-primary-foreground",
	warning: "bg-amber-500 text-black",
	danger: "bg-destructive text-destructive-foreground",
};

const RailButton = memo(function RailButton({
	item,
}: Readonly<{ item: IBoardRailItem }>) {
	const badgeLabel =
		typeof item.badge === "number" && item.badge > 0
			? (item.badgeLabel ?? String(item.badge))
			: undefined;
	const label = badgeLabel ? `${item.title}: ${badgeLabel}` : item.title;
	const className = cn(
		"relative flex size-9 shrink-0 items-center justify-center rounded-lg border border-transparent text-muted-foreground transition-colors",
		"hover:bg-sidebar-accent hover:text-sidebar-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-offset-1 focus-visible:ring-offset-sidebar",
		item.active && "border-primary/15 bg-primary/10 text-primary shadow-xs",
		item.href && "mb-1",
	);
	const content = (
		<>
			<span className="[&>svg]:size-4.5">{item.icon}</span>
			{typeof item.badge === "number" && item.badge > 0 && (
				<span
					aria-hidden="true"
					className={cn(
						"absolute -right-0.5 -top-0.5 min-w-3.5 rounded-full px-1 text-[9px] font-semibold leading-3.5 tabular-nums ring-2 ring-sidebar",
						BADGE_TONE[item.badgeTone ?? "default"],
					)}
				>
					{item.badge > 99 ? "99+" : item.badge}
				</span>
			)}
		</>
	);

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				{item.href ? (
					<Link href={item.href} aria-label={label} className={className}>
						{content}
					</Link>
				) : (
					<button
						type="button"
						aria-label={label}
						aria-pressed={Boolean(item.active)}
						onClick={item.onSelect}
						className={className}
					>
						{content}
					</button>
				)}
			</TooltipTrigger>
			<TooltipContent side="right" className="flex flex-col gap-1">
				<span className="flex items-center gap-2">
					{item.title}
					{item.shortcut && (
						<span className="font-mono text-[10px] text-muted-foreground">
							{item.shortcut}
						</span>
					)}
				</span>
				{badgeLabel && <span className="text-[11px]">{badgeLabel}</span>}
			</TooltipContent>
		</Tooltip>
	);
});

/**
 * The one place every board surface is reachable from — replaces the floating
 * dock. It is a column in the layout, so it can neither overlap the canvas nor
 * be painted over by a panel, and every entry reports whether its view is open.
 */
export const BoardActivityRail = memo(function BoardActivityRail({
	top,
	bottom,
	footer,
}: Readonly<{
	top: IBoardRailItem[];
	bottom: IBoardRailItem[];
	/**
	 * Pinned below the bottom commands. Holds what the rail cannot express as a
	 * command — the account avatar, whose icon is an image rather than a
	 * `LucideIcon`.
	 */
	footer?: ReactNode;
}>) {
	return (
		<nav
			aria-label="Board surfaces"
			className="flex w-11 shrink-0 flex-col items-center gap-1 border-r border-sidebar-border bg-sidebar py-2"
		>
			{/* Only the view list scrolls. A short viewport must not be able to push
			    the account or the pinned commands off the bottom of the window. */}
			<div className="no-scrollbar flex min-h-0 w-full flex-1 flex-col items-center gap-1 overflow-y-auto py-0.5">
				{top.map((item) => (
					<RailButton key={item.id} item={item} />
				))}
			</div>
			{bottom.map((item) => (
				<RailButton key={item.id} item={item} />
			))}
			{footer && (
				<div className="mt-1 flex w-full shrink-0 flex-col items-center border-t border-sidebar-border pt-2">
					{footer}
				</div>
			)}
		</nav>
	);
});
