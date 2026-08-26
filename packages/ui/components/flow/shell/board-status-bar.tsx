"use client";

import type { ReactNode } from "react";
import { memo } from "react";
import { cn } from "../../../lib/utils";
import { Popover, PopoverContent, PopoverTrigger } from "../../ui/popover";

/**
 * One item of board state, and — where the state is editable — the thing that
 * edits it. Board settings have no dialog any more: each lives beside the value
 * it changes, so the version popover opens off the version, and the name and
 * description off the name.
 */
export const BoardStatusItem = memo(function BoardStatusItem({
	icon,
	children,
	tone = "default",
	title,
	onClick,
	className,
	popover,
	popoverAlign = "start",
	popoverClassName,
}: Readonly<{
	icon?: ReactNode;
	children?: ReactNode;
	tone?: "default" | "muted" | "warning" | "danger" | "accent";
	title?: string;
	onClick?: () => void;
	className?: string;
	/** Rendered in a popover anchored to this item; makes the item a trigger. */
	popover?: ReactNode;
	popoverAlign?: "start" | "center" | "end";
	popoverClassName?: string;
}>) {
	const tones: Record<string, string> = {
		default: "text-foreground/80",
		muted: "text-muted-foreground",
		warning: "text-amber-500",
		danger: "text-destructive",
		accent: "text-primary",
	};
	const content = (
		<>
			{icon && <span className="[&>svg]:size-3">{icon}</span>}
			{children}
		</>
	);
	const classes = cn(
		"flex items-center gap-1.5 whitespace-nowrap px-1.5 py-0.5 text-[11px] leading-none",
		tones[tone],
		(onClick || popover) &&
			"rounded-sm hover:bg-accent hover:text-accent-foreground",
		className,
	);
	if (popover) {
		return (
			<Popover>
				<PopoverTrigger asChild>
					<button type="button" title={title} className={classes}>
						{content}
					</button>
				</PopoverTrigger>
				<PopoverContent
					side="top"
					align={popoverAlign}
					sideOffset={8}
					className={cn("w-80 p-3", popoverClassName)}
				>
					{popover}
				</PopoverContent>
			</Popover>
		);
	}
	if (!onClick) {
		return (
			<span title={title} className={classes}>
				{content}
			</span>
		);
	}
	return (
		<button type="button" title={title} onClick={onClick} className={classes}>
			{content}
		</button>
	);
});

/**
 * The permanent home for board state. Everything here used to be `fixed` to the
 * viewport's right edge — the same edge the side panels occupy — so it floated
 * over whichever panel header happened to be underneath.
 */
export const BoardStatusBar = memo(function BoardStatusBar({
	left,
	right,
}: Readonly<{ left?: ReactNode; right?: ReactNode }>) {
	return (
		<footer className="flex h-6 shrink-0 items-center gap-1 border-t bg-muted/30 px-1.5 text-[11px] text-muted-foreground">
			<div className="flex min-w-0 items-center gap-1 overflow-hidden">
				{left}
			</div>
			<span className="flex-1" />
			<div className="flex shrink-0 items-center gap-1">{right}</div>
		</footer>
	);
});
