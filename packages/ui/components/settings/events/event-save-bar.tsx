"use client";

import {
	Loader2Icon,
	SaveIcon,
	TriangleAlertIcon,
	Undo2Icon,
} from "lucide-react";
import { cn } from "../../../lib/utils";
import { Button } from "../../ui/button";

/**
 * Unsaved-changes bar for the event editor.
 *
 * Sticky rather than fixed: a fixed bar is positioned against the visual
 * viewport, so on mobile the soft keyboard and the browser chrome push it out
 * of reach exactly while the user is editing. Sticky keeps it inside the
 * config scroll container, which also stops it from spanning the sidebar.
 * On mobile it pins to the top, on wider screens it stays the familiar bottom
 * action bar.
 */
export function EventSaveBar({
	placement,
	isDirty,
	isSaving,
	error,
	onSave,
	onDiscard,
}: Readonly<{
	placement: "top" | "bottom";
	isDirty: boolean;
	isSaving: boolean;
	error?: string | null;
	onSave: () => void;
	onDiscard: () => void;
}>) {
	const isTop = placement === "top";
	const status = isSaving
		? "Saving…"
		: isDirty
			? "Unsaved changes"
			: "Editing mode";

	return (
		<div
			className={cn(
				"sticky z-30",
				isTop ? "top-0 pb-3" : "bottom-0 pt-3 pb-safe",
			)}
		>
			<div
				className={cn(
					"flex items-center gap-3 rounded-xl border bg-background/95 px-3 py-2.5 shadow-floating backdrop-blur supports-backdrop-filter:bg-background/85",
					error ? "border-destructive/50" : "border-border",
				)}
			>
				<div className="flex min-w-0 flex-1 items-center gap-2">
					{error ? (
						<TriangleAlertIcon className="h-4 w-4 shrink-0 text-destructive" />
					) : (
						<span
							className={cn(
								"h-2 w-2 shrink-0 rounded-full",
								isDirty ? "bg-amber-500" : "bg-muted-foreground/40",
							)}
						/>
					)}
					<div className="min-w-0">
						<p
							className={cn(
								"truncate text-sm font-medium",
								error && "text-destructive",
							)}
						>
							{error ?? status}
						</p>
						{!error && isDirty && !isSaving && (
							<p className="hidden truncate text-xs text-muted-foreground sm:block">
								Your edits are not live until you save.
							</p>
						)}
					</div>
				</div>

				<div className="flex shrink-0 items-center gap-2">
					<Button
						variant="outline"
						onClick={onDiscard}
						disabled={isSaving}
						aria-label="Discard changes"
						className="h-10 sm:h-9"
					>
						<Undo2Icon className="h-4 w-4" />
						<span className="hidden sm:inline">Discard</span>
					</Button>
					<Button
						onClick={onSave}
						disabled={!isDirty || isSaving}
						className="h-10 sm:h-9"
					>
						{isSaving ? (
							<Loader2Icon className="h-4 w-4 animate-spin" />
						) : (
							<SaveIcon className="h-4 w-4" />
						)}
						<span className="sm:hidden">{isSaving ? "Saving…" : "Save"}</span>
						<span className="hidden sm:inline">
							{isSaving ? "Saving…" : "Save Changes"}
						</span>
					</Button>
				</div>
			</div>
		</div>
	);
}
