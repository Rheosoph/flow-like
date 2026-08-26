"use client";

import type { ReactNode } from "react";
import { memo } from "react";
import { cn } from "../../../lib/utils";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/tooltip";

export interface IBoardEditorAction {
	id: string;
	title: string;
	icon: ReactNode;
	/** Shown beside the icon once there is room; the title is always the tooltip. */
	label?: string;
	active?: boolean;
	disabled?: boolean;
	shortcut?: string;
	onSelect: () => void;
}

/**
 * Actions that act on what the editor is showing, pinned to the right of the file
 * tabs — the split toggle, auto layout, templates.
 *
 * They live here rather than in the activity rail because the rail toggles
 * *regions* of the shell, while these change the *document*. Keeping the two
 * apart is what stops the rail drifting back into a dock of unrelated verbs.
 */
export const BoardEditorActions = memo(function BoardEditorActions({
	actions,
}: Readonly<{ actions: IBoardEditorAction[] }>) {
	if (actions.length === 0) return null;
	return (
		<div className="flex shrink-0 items-center gap-0.5">
			{actions.map((action) => (
				<Tooltip key={action.id}>
					<TooltipTrigger asChild>
						<button
							type="button"
							onClick={action.onSelect}
							disabled={action.disabled}
							aria-pressed={Boolean(action.active)}
							aria-label={action.title}
							className={cn(
								"flex items-center gap-1 rounded-sm px-1.5 py-0.5 text-xs text-muted-foreground transition-colors",
								"hover:bg-accent hover:text-foreground disabled:pointer-events-none disabled:opacity-40",
								action.active && "bg-accent text-foreground",
							)}
						>
							<span className="[&>svg]:size-3.5">{action.icon}</span>
							{action.label && (
								<span className="hidden lg:inline">{action.label}</span>
							)}
						</button>
					</TooltipTrigger>
					<TooltipContent side="bottom" className="flex items-center gap-2">
						{action.title}
						{action.shortcut && (
							<span className="font-mono text-[10px] text-muted-foreground">
								{action.shortcut}
							</span>
						)}
					</TooltipContent>
				</Tooltip>
			))}
		</div>
	);
});
