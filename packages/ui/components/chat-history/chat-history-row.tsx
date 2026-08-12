"use client";

import {
	CheckIcon,
	MessageSquareIcon,
	MoreHorizontalIcon,
	PencilIcon,
	PinIcon,
	PinOffIcon,
	Trash2Icon,
	XIcon,
} from "lucide-react";
import { memo, useCallback, useEffect, useRef, useState } from "react";
import { cn } from "../../lib/utils";
import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "../ui/alert-dialog";
import { Button } from "../ui/button";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuSeparator,
	DropdownMenuTrigger,
} from "../ui/dropdown-menu";
import { Input } from "../ui/input";
import { RelativeTime } from "../ui/relative-time";
import type { IHistoryEntry } from "./chat-history-types";
import { highlightMatch } from "./highlight-match";

interface ChatHistoryRowProps {
	entry: IHistoryEntry;
	active: boolean;
	query: string;
	/** Touch surfaces keep the actions visible; pointer surfaces reveal them on hover/focus. */
	comfortable: boolean;
	/** Rename state is owned by the list, so only one row can be in rename mode at a time. */
	renaming: boolean;
	onRenamingChange: (renaming: boolean) => void;
	onSelect: (id: string) => void | Promise<void>;
	onTogglePin?: (id: string, pinned: boolean) => void | Promise<void>;
	onRename?: (id: string, title: string) => void | Promise<void>;
	onDelete?: (id: string) => void | Promise<void>;
}

/**
 * One conversation row. Memoized because the list re-renders on every keystroke in the search
 * field, and each row formats two timestamps and runs a regex split.
 */
export const ChatHistoryRow = memo(function ChatHistoryRow({
	entry,
	active,
	query,
	comfortable,
	renaming,
	onRenamingChange,
	onSelect,
	onTogglePin,
	onRename,
	onDelete,
}: Readonly<ChatHistoryRowProps>) {
	const [draft, setDraft] = useState(entry.title);
	const [confirmingDelete, setConfirmingDelete] = useState(false);
	const inputRef = useRef<HTMLInputElement>(null);
	const pinned = Boolean(entry.pinnedAt);
	const Icon = entry.icon ?? MessageSquareIcon;

	// Read through a ref so the seed happens only when rename opens: a title that changes underneath
	// an open editor must not wipe what the user has typed so far.
	const titleRef = useRef(entry.title);
	titleRef.current = entry.title;

	useEffect(() => {
		if (!renaming) return;
		setDraft(titleRef.current);
		// Select-all so the common case (replace the auto-generated title) is a single keystroke.
		requestAnimationFrame(() => inputRef.current?.select());
	}, [renaming]);

	const commitRename = useCallback(() => {
		onRenamingChange(false);
		const next = draft.trim();
		if (next && next !== entry.title) void onRename?.(entry.id, next);
	}, [draft, entry.id, entry.title, onRename, onRenamingChange]);

	const handleRenameKeyDown = useCallback(
		(event: React.KeyboardEvent<HTMLInputElement>) => {
			if (event.key === "Enter") {
				event.preventDefault();
				commitRename();
			} else if (event.key === "Escape") {
				// The host layer's own Escape guard is what stops the surrounding Popover/Sheet from
				// dismissing; this only ends the rename.
				event.preventDefault();
				onRenamingChange(false);
			}
		},
		[commitRename, onRenamingChange],
	);

	if (renaming) {
		return (
			<li className="flex items-center gap-1 rounded-lg bg-muted/50 px-1.5 py-1">
				<Input
					ref={inputRef}
					value={draft}
					onChange={(event) => setDraft(event.target.value)}
					onKeyDown={handleRenameKeyDown}
					onBlur={commitRename}
					aria-label="Conversation title"
					className="h-9 flex-1 border-transparent bg-transparent px-1.5 text-[16px] shadow-none focus-visible:ring-1 md:h-7 md:text-sm"
				/>
				<Button
					variant="ghost"
					size="icon"
					className="size-8 shrink-0 rounded-md md:size-7"
					aria-label="Save title"
					onMouseDown={(event) => event.preventDefault()}
					onClick={commitRename}
				>
					<CheckIcon className="size-3.5" />
				</Button>
				<Button
					variant="ghost"
					size="icon"
					className="size-8 shrink-0 rounded-md md:size-7"
					aria-label="Cancel rename"
					onMouseDown={(event) => event.preventDefault()}
					onClick={() => onRenamingChange(false)}
				>
					<XIcon className="size-3.5" />
				</Button>
			</li>
		);
	}

	const actionsVisible = comfortable
		? "opacity-100"
		: "opacity-0 transition-opacity group-hover:opacity-100 group-focus-within:opacity-100 data-[state=open]:opacity-100";

	return (
		<li
			className={cn(
				"group relative flex items-center gap-0.5 rounded-lg pr-1 transition-colors",
				active ? "bg-primary/10" : "hover:bg-muted/50",
			)}
		>
			{active && (
				<span
					aria-hidden
					className="absolute inset-y-2 left-0 w-0.5 rounded-full bg-primary"
				/>
			)}
			<button
				type="button"
				onClick={() => void onSelect(entry.id)}
				className={cn(
					"flex min-w-0 flex-1 items-center gap-2.5 rounded-lg px-2.5 text-left outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
					comfortable ? "min-h-12 py-2.5" : "py-2",
				)}
			>
				<span
					className={cn(
						"grid size-7 shrink-0 place-items-center rounded-md",
						active
							? "bg-primary/20 text-primary"
							: "bg-muted text-muted-foreground",
					)}
				>
					<Icon className="size-3.5" />
				</span>
				<span className="flex min-w-0 flex-1 flex-col">
					<span className="flex items-center gap-1.5">
						<span className="truncate text-sm font-medium">
							{highlightMatch(entry.title, query)}
						</span>
						{entry.streaming && (
							<span
								aria-label="Responding"
								className="size-1.5 shrink-0 animate-pulse rounded-full bg-primary"
							/>
						)}
					</span>
					<span className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
						<RelativeTime value={entry.updatedAt} />
						{entry.subtitle && (
							<>
								<span aria-hidden>·</span>
								<span className="truncate">{entry.subtitle}</span>
							</>
						)}
					</span>
				</span>
			</button>

			{onTogglePin && !comfortable && (
				<Button
					variant="ghost"
					size="icon"
					className={cn(
						"size-7 shrink-0 rounded-md text-muted-foreground hover:text-foreground",
						// A pinned row keeps its toggle visible, or unpinning is undiscoverable.
						pinned ? "opacity-100" : actionsVisible,
					)}
					aria-label={pinned ? "Unpin conversation" : "Pin conversation"}
					onClick={() => void onTogglePin(entry.id, !pinned)}
				>
					{pinned ? (
						<PinIcon className="size-3.5 fill-current text-primary" />
					) : (
						<PinIcon className="size-3.5" />
					)}
				</Button>
			)}

			{(onTogglePin || onRename || onDelete) && (
				<DropdownMenu>
					<DropdownMenuTrigger asChild>
						<Button
							variant="ghost"
							size="icon"
							className={cn(
								"shrink-0 rounded-md text-muted-foreground hover:text-foreground",
								comfortable ? "size-9" : "size-7",
								actionsVisible,
							)}
							aria-label="Conversation options"
						>
							<MoreHorizontalIcon className="size-3.5" />
						</Button>
					</DropdownMenuTrigger>
					<DropdownMenuContent align="end" className="z-[10002] w-44">
						{onTogglePin && (
							<DropdownMenuItem
								onSelect={() => void onTogglePin(entry.id, !pinned)}
							>
								{pinned ? (
									<PinOffIcon className="size-3.5" />
								) : (
									<PinIcon className="size-3.5" />
								)}
								{pinned ? "Unpin" : "Pin"}
							</DropdownMenuItem>
						)}
						{onRename && (
							<DropdownMenuItem onSelect={() => onRenamingChange(true)}>
								<PencilIcon className="size-3.5" />
								Rename
							</DropdownMenuItem>
						)}
						{onDelete && (
							<>
								<DropdownMenuSeparator />
								<DropdownMenuItem
									variant="destructive"
									onSelect={() => setConfirmingDelete(true)}
								>
									<Trash2Icon className="size-3.5" />
									Delete
								</DropdownMenuItem>
							</>
						)}
					</DropdownMenuContent>
				</DropdownMenu>
			)}

			{onDelete && (
				<AlertDialog open={confirmingDelete} onOpenChange={setConfirmingDelete}>
					<AlertDialogContent className="z-[10002]">
						<AlertDialogHeader>
							<AlertDialogTitle>Delete this conversation?</AlertDialogTitle>
							<AlertDialogDescription>
								“{entry.title}” and all of its messages will be removed from
								this device. This cannot be undone.
								{entry.streaming &&
									" This conversation is still responding — the run will be discarded."}
							</AlertDialogDescription>
						</AlertDialogHeader>
						<AlertDialogFooter>
							<AlertDialogCancel>Cancel</AlertDialogCancel>
							<AlertDialogAction
								className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
								onClick={() => void onDelete(entry.id)}
							>
								Delete
							</AlertDialogAction>
						</AlertDialogFooter>
					</AlertDialogContent>
				</AlertDialog>
			)}
		</li>
	);
});
