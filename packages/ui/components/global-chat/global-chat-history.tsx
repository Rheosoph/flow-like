"use client";

import { useTranslation } from "@flow-like/locales";
import { useLiveQuery } from "dexie-react-hooks";
import { HistoryIcon, SquarePenIcon } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { useIsMobile } from "../../hooks/use-mobile";
import { cn } from "../../lib/utils";
import {
	GLOBAL_CHAT_APP_ID,
	globalChatDb,
} from "../../state/global-chat/global-chat-db";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import {
	deleteGlobalChatConversation,
	renameGlobalChatSession,
	restoreGlobalChatConversation,
	setGlobalChatSessionPinned,
} from "../../state/global-chat/global-chat-stream";
import { ChatHistoryList } from "../chat-history/chat-history-list";
import type { IHistoryEntry } from "../chat-history/chat-history-types";
import { buildSearchCorpus } from "../chat-history/use-history-search";
import { Button } from "../ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";
import {
	Sheet,
	SheetContent,
	SheetHeader,
	SheetTitle,
	SheetTrigger,
} from "../ui/sheet";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";

/** Recency window for unpinned conversations. Pinned rows are fetched separately and never cut. */
const RECENT_LIMIT = 200;

interface GlobalChatHistoryProps {
	className?: string;
}

/**
 * Global chat history: a New-chat action plus a searchable, pinnable conversation list.
 *
 * The list renders in a Popover on pointer devices and a bottom Sheet on touch, since a 320px
 * popover anchored to a header button is unusable with a thumb.
 */
export function GlobalChatHistory({
	className,
}: Readonly<GlobalChatHistoryProps>) {
	const { t } = useTranslation("chat");
	const [open, setOpen] = useState(false);
	const isMobile = useIsMobile();

	// Switching chats no longer blocks on a live turn: runs are keyed by run id and keep streaming
	// (and finalizing into IndexedDB) in whatever conversation they belong to, reappearing in place
	// when the user comes back.
	const activeConversationId = useGlobalChatStore(
		(s) => s.activeConversationId,
	);
	const newConversation = useGlobalChatStore((s) => s.newConversation);
	// A comma-joined key, not the `runs` record: `runs` gets a fresh identity on every streamed
	// chunk, which would re-derive `entries` and rebuild the whole MiniSearch index per token.
	// This string only changes when a conversation actually starts or finishes a run.
	const streamingKey = useGlobalChatStore((s) =>
		Array.from(new Set(Object.values(s.runs).map((run) => run.conversationId)))
			.sort()
			.join(","),
	);

	// Two queries rather than one: a single `limit()` would be applied before the appId filter (so
	// migrated board/ui rows eat slots) and would drop pinned-but-old conversations entirely.
	const pinnedSessions = useLiveQuery(
		() => globalChatDb.sessions.where("pinnedAt").above(0).toArray(),
		[],
	);
	const recentSessions = useLiveQuery(
		() =>
			globalChatDb.sessions
				.orderBy("updatedAt")
				.reverse()
				.filter((session) => session.appId === GLOBAL_CHAT_APP_ID)
				.limit(RECENT_LIMIT)
				.toArray(),
		[],
	);

	const sessions = useMemo(() => {
		if (!pinnedSessions || !recentSessions) return undefined;
		const pinned = pinnedSessions.filter(
			(session) => session.appId === GLOBAL_CHAT_APP_ID,
		);
		const pinnedIds = new Set(pinned.map((session) => session.id));
		return [
			...pinned,
			...recentSessions.filter((session) => !pinnedIds.has(session.id)),
		];
	}, [pinnedSessions, recentSessions]);

	const sessionIds = useMemo(
		() => (sessions ?? []).map((session) => session.id),
		[sessions],
	);
	const sessionIdKey = sessionIds.join(",");

	// One indexed range scan over every listed conversation, not one query per row — on desktop each
	// IndexedDB op goes through the SQLite shim, so an N+1 here would be felt directly. Gated on the
	// user actually searching, so merely opening the panel never pulls every message it has.
	const [searching, setSearching] = useState(false);
	const [renaming, setRenaming] = useState(false);
	const messages = useLiveQuery(
		() =>
			searching && sessionIds.length > 0
				? globalChatDb.messages.where("sessionId").anyOf(sessionIds).toArray()
				: [],
		[searching, sessionIdKey],
	);

	const streamingIds = useMemo(
		() => new Set(streamingKey ? streamingKey.split(",") : []),
		[streamingKey],
	);

	const entries = useMemo<IHistoryEntry[] | undefined>(() => {
		if (!sessions) return undefined;
		const corpus = buildSearchCorpus(messages);
		return sessions.map((session) => ({
			id: session.id,
			title: session.summarization || "Untitled conversation",
			updatedAt: session.updatedAt,
			pinnedAt: session.pinnedAt,
			streaming: streamingIds.has(session.id),
			searchBody: corpus.get(session.id) ?? "",
		}));
	}, [sessions, messages, streamingIds]);

	// Shared restore path: settles stale mid-stream checkpoints (no eternal spinners) and
	// re-attaches runs of this conversation that are still streaming in Rust.
	const handleSelect = useCallback(async (sessionId: string) => {
		await restoreGlobalChatConversation(sessionId);
		setOpen(false);
	}, []);

	const handleNew = useCallback(() => {
		newConversation();
		setOpen(false);
	}, [newConversation]);

	const list = (
		<ChatHistoryList
			entries={entries}
			activeId={activeConversationId}
			onSelect={handleSelect}
			onNew={handleNew}
			onTogglePin={setGlobalChatSessionPinned}
			onRename={renameGlobalChatSession}
			onDelete={deleteGlobalChatConversation}
			density={isMobile ? "comfortable" : "compact"}
			onSearchActiveChange={setSearching}
			onRenamingChange={setRenaming}
			emptyDescription="Ask FlowPilot something and the conversation shows up here."
			className={isMobile ? "h-full" : "max-h-[min(70vh,480px)]"}
		/>
	);

	// Radix dismisses on Escape from a document capture listener, which fires before the rename
	// input's own handler — without this the key would tear down the whole surface mid-rename.
	const guardEscape = (event: KeyboardEvent) => {
		if (renaming) event.preventDefault();
	};

	const trigger = (
		<Button
			variant="ghost"
			size="icon"
			className="h-9 w-9 rounded-lg md:h-8 md:w-8"
			aria-label={t('chatHistory', 'Chat history')}
		>
			<HistoryIcon className="size-4" />
		</Button>
	);

	return (
		<div className={cn("flex shrink-0 items-center gap-0.5", className)}>
			<Tooltip>
				<TooltipTrigger asChild>
					<Button
						variant="ghost"
						size="icon"
						className="h-9 w-9 rounded-lg md:h-8 md:w-8"
						aria-label={t('newChat', 'New chat')}
						onClick={newConversation}
					>
						<SquarePenIcon className="size-4" />
					</Button>
				</TooltipTrigger>
				<TooltipContent side="bottom" className="text-xs">
					{t('newChat', 'New chat')}
				</TooltipContent>
			</Tooltip>

			{isMobile ? (
				<Sheet open={open} onOpenChange={setOpen}>
					<SheetTrigger asChild>{trigger}</SheetTrigger>
					{/* The docked chat overlay sits at z-9999, so the Sheet and its backdrop have to
					    be raised explicitly or they render behind it. */}
					<SheetContent
						side="bottom"
						overlayClassName="z-[10000]"
						onEscapeKeyDown={guardEscape}
						className="z-[10001] flex h-[85dvh] max-h-[85dvh] flex-col overflow-hidden rounded-t-2xl p-0"
					>
						<SheetHeader className="shrink-0 px-4 pb-0 pt-4">
							<SheetTitle className="text-base">{t('conversations', 'Conversations')}</SheetTitle>
						</SheetHeader>
						{/* SheetContent wraps children in a flex column; without min-h-0 this never scrolls. */}
						<div className="flex min-h-0 flex-1 flex-col">{list}</div>
					</SheetContent>
				</Sheet>
			) : (
				<Popover open={open} onOpenChange={setOpen}>
					<Tooltip>
						<TooltipTrigger asChild>
							<PopoverTrigger asChild>{trigger}</PopoverTrigger>
						</TooltipTrigger>
						<TooltipContent side="bottom" className="text-xs">
							{t('history', 'History')}
						</TooltipContent>
					</Tooltip>
					<PopoverContent
						align="end"
						sideOffset={8}
						onEscapeKeyDown={guardEscape}
						className="z-[10000] w-[380px] max-w-[calc(100vw-2rem)] overflow-hidden p-0 shadow-floating"
					>
						{list}
					</PopoverContent>
				</Popover>
			)}
		</div>
	);
}
