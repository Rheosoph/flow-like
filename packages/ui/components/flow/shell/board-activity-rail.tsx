"use client";

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
	badgeTone?: "default" | "warning" | "danger";
	shortcut?: string;
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
	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<button
					type="button"
					aria-label={item.title}
					aria-pressed={Boolean(item.active)}
					onClick={item.onSelect}
					className={cn(
						"relative flex h-10 w-11 shrink-0 items-center justify-center text-muted-foreground transition-colors",
						"hover:text-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring",
						item.active && "text-foreground",
					)}
				>
					<span
						className={cn(
							"absolute left-0 top-1/2 h-5 w-0.5 -translate-y-1/2 rounded-r bg-primary opacity-0 transition-opacity",
							item.active && "opacity-100",
						)}
					/>
					<span className="[&>svg]:size-4.5">{item.icon}</span>
					{typeof item.badge === "number" && item.badge > 0 && (
						<span
							className={cn(
								"absolute right-1 top-1 min-w-3.5 rounded-full px-1 text-[9px] font-semibold leading-3.5 tabular-nums",
								BADGE_TONE[item.badgeTone ?? "default"],
							)}
						>
							{item.badge > 99 ? "99+" : item.badge}
						</span>
					)}
				</button>
			</TooltipTrigger>
			<TooltipContent side="right" className="flex items-center gap-2">
				{item.title}
				{item.shortcut && (
					<span className="font-mono text-[10px] text-muted-foreground">
						{item.shortcut}
					</span>
				)}
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
}: Readonly<{ top: IBoardRailItem[]; bottom: IBoardRailItem[] }>) {
	return (
		<nav
			aria-label="Board surfaces"
			className="flex w-11 shrink-0 flex-col items-center border-r bg-muted/20 py-1"
		>
			{top.map((item) => (
				<RailButton key={item.id} item={item} />
			))}
			<span className="flex-1" />
			{bottom.map((item) => (
				<RailButton key={item.id} item={item} />
			))}
		</nav>
	);
});
