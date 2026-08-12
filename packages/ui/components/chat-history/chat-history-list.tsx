"use client";

import {
	MessageSquareIcon,
	PinIcon,
	SearchIcon,
	SquarePenIcon,
	XIcon,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { cn } from "../../lib/utils";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
import { Skeleton } from "../ui/skeleton";
import { ChatHistoryRow } from "./chat-history-row";
import type { IChatHistoryListProps } from "./chat-history-types";
import { groupHistoryByDate } from "./group-history";
import { useHistorySearch } from "./use-history-search";

function HistorySkeleton() {
	return (
		<div className="space-y-1 p-1.5" aria-hidden>
			{[0, 1, 2, 3].map((row) => (
				<div key={row} className="flex items-center gap-2.5 px-2.5 py-2">
					<Skeleton className="size-7 shrink-0 rounded-md" />
					<div className="flex-1 space-y-1.5">
						<Skeleton
							className="h-3.5 rounded"
							style={{ width: `${70 - row * 8}%` }}
						/>
						<Skeleton className="h-2.5 w-16 rounded" />
					</div>
				</div>
			))}
		</div>
	);
}

/**
 * Presentational conversation history: search, pinned + date sections, and per-row pin / rename /
 * delete. Deliberately owns no data access — every surface (global chat, per-app chat, FlowPilot)
 * normalizes its own store into `IHistoryEntry[]` and passes the mutations it supports.
 */
export function ChatHistoryList({
	entries,
	activeId,
	onSelect,
	onNew,
	onTogglePin,
	onRename,
	onDelete,
	searchPlaceholder = "Search conversations…",
	density = "compact",
	header,
	emptyTitle = "No conversations yet",
	emptyDescription = "Start a chat and it will show up here.",
	onSearchActiveChange,
	onRenamingChange,
	className,
}: Readonly<IChatHistoryListProps>) {
	const [query, setQuery] = useState("");
	const [renamingId, setRenamingId] = useState<string | null>(null);
	const comfortable = density === "comfortable";
	const { results, appliedQuery, bodySearchEnabled } = useHistorySearch(
		entries,
		query,
	);

	// Held in refs so an inline callback at the call site cannot turn these into per-render churn.
	const notifySearch = useRef(onSearchActiveChange);
	notifySearch.current = onSearchActiveChange;
	const notifyRenaming = useRef(onRenamingChange);
	notifyRenaming.current = onRenamingChange;

	// Message bodies are only worth loading once the user is actually searching. Callers subscribe
	// here so they can keep that (potentially large) Dexie read out of the idle render.
	useEffect(() => {
		notifySearch.current?.(bodySearchEnabled);
	}, [bodySearchEnabled]);

	useEffect(() => {
		notifyRenaming.current?.(renamingId !== null);
	}, [renamingId]);

	// Reset both on unmount: the list is unmounted by its host (a Popover/Sheet closing) while the
	// host itself stays mounted, so a flag left latched here would keep a Dexie subscription — and
	// the host's Escape guard — alive for the rest of the session.
	useEffect(
		() => () => {
			notifySearch.current?.(false);
			notifyRenaming.current?.(false);
		},
		[],
	);

	const groups = useMemo(
		// Search results are relevance-ranked, so date sections would scatter the best match down
		// the list. While searching, show one flat ranked list instead — and no group at all when
		// nothing matched, so the caller-visible empty state is what renders.
		() =>
			appliedQuery
				? results.length > 0
					? [{ key: "results", label: "Results", entries: results }]
					: []
				: groupHistoryByDate(results),
		[appliedQuery, results],
	);

	const handleSelect = useCallback(
		(id: string) => {
			setQuery("");
			setRenamingId(null);
			return onSelect(id);
		},
		[onSelect],
	);

	const loading = entries === undefined;
	// Keyed off "has any history at all", never off the result count — hiding the field as soon as
	// a query narrows the list would trap the user with no way to edit what they typed.
	const showSearch = loading || (entries?.length ?? 0) > 0;

	return (
		<div className={cn("flex min-h-0 flex-col", className)}>
			<div className="shrink-0 border-b border-border/50 p-1.5">
				<div className="flex items-center gap-1.5">
					{showSearch && (
						<div className="relative min-w-0 flex-1">
							<SearchIcon className="pointer-events-none absolute left-2.5 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
							<Input
								value={query}
								onChange={(event) => setQuery(event.target.value)}
								placeholder={searchPlaceholder}
								aria-label="Search conversations"
								// 16px on mobile is mandatory: anything smaller makes iOS Safari zoom the
								// viewport on focus and the surface never zooms back out.
								className={cn(
									"rounded-lg border-border/50 bg-muted/40 pl-8 pr-8 text-[16px] md:text-sm",
									comfortable ? "h-11" : "h-9 md:h-8",
								)}
							/>
							{query && (
								<Button
									variant="ghost"
									size="icon"
									className="absolute right-1 top-1/2 size-6 -translate-y-1/2 rounded-md text-muted-foreground"
									aria-label="Clear search"
									onClick={() => setQuery("")}
								>
									<XIcon className="size-3.5" />
								</Button>
							)}
						</div>
					)}
					{onNew && (
						<Button
							variant="outline"
							size="icon"
							onClick={onNew}
							aria-label="New chat"
							className={cn(
								"shrink-0 rounded-lg",
								comfortable ? "size-11" : "size-9 md:size-8",
							)}
						>
							<SquarePenIcon className="size-3.5" />
						</Button>
					)}
				</div>
				{header && (
					<div className="px-1 pt-1.5 text-[11px] text-muted-foreground">
						{header}
					</div>
				)}
			</div>

			<div className="min-h-0 flex-1 touch-pan-y overflow-y-auto overscroll-contain">
				{loading && <HistorySkeleton />}

				{!loading && groups.length === 0 && (
					<div className="flex flex-col items-center justify-center gap-1 px-6 py-12 text-center">
						<span className="mb-1 grid size-10 place-items-center rounded-xl border border-border/50 bg-muted/60">
							<MessageSquareIcon className="size-4 text-muted-foreground" />
						</span>
						<p className="text-sm font-medium">
							{appliedQuery ? "No matches" : emptyTitle}
						</p>
						<p className="text-xs text-muted-foreground">
							{appliedQuery
								? `Nothing matches “${appliedQuery}”.`
								: emptyDescription}
						</p>
						{appliedQuery ? (
							<Button
								variant="ghost"
								size="sm"
								className="mt-2 h-8 text-xs"
								onClick={() => setQuery("")}
							>
								Clear search
							</Button>
						) : (
							onNew && (
								<Button
									variant="outline"
									size="sm"
									className="mt-2 h-8 gap-1.5 text-xs"
									onClick={onNew}
								>
									<SquarePenIcon className="size-3.5" />
									New chat
								</Button>
							)
						)}
					</div>
				)}

				{!loading &&
					groups.map((group) => (
						<section key={group.key} className="px-1.5 pb-1 pt-2">
							<h3 className="flex items-center gap-1.5 px-2 pb-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
								{group.pinned && <PinIcon className="size-3" />}
								{group.label}
								<span className="ml-auto tabular-nums opacity-70">
									{group.entries.length}
								</span>
							</h3>
							<ul className="space-y-0.5">
								{group.entries.map((entry) => (
									<ChatHistoryRow
										key={entry.id}
										entry={entry}
										active={entry.id === activeId}
										query={appliedQuery}
										comfortable={comfortable}
										renaming={renamingId === entry.id}
										onRenamingChange={(next) =>
											setRenamingId(next ? entry.id : null)
										}
										onSelect={handleSelect}
										onTogglePin={onTogglePin}
										onRename={onRename}
										onDelete={onDelete}
									/>
								))}
							</ul>
						</section>
					))}
			</div>
		</div>
	);
}
