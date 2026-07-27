"use client";

import { useLiveQuery } from "dexie-react-hooks";
import { HistoryIcon, SquarePenIcon, Trash2Icon } from "lucide-react";
import { useCallback, useState } from "react";
import {
	Button,
	Popover,
	PopoverContent,
	PopoverTrigger,
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "../../index";
import {
	GLOBAL_CHAT_APP_ID,
	globalChatDb,
} from "../../state/global-chat/global-chat-db";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { restoreGlobalChatConversation } from "../../state/global-chat/global-chat-stream";

function relativeTime(timestamp: number): string {
	const delta = Date.now() - timestamp;
	const minutes = Math.floor(delta / 60_000);
	if (minutes < 1) return "just now";
	if (minutes < 60) return `${minutes}m ago`;
	const hours = Math.floor(minutes / 60);
	if (hours < 24) return `${hours}h ago`;
	const days = Math.floor(hours / 24);
	if (days < 7) return `${days}d ago`;
	return new Date(timestamp).toLocaleDateString();
}

/**
 * Global chat history: a New-chat action plus a popover listing persisted conversations
 * (from the GlobalChat Dexie DB) with resume and delete.
 */
export function GlobalChatHistory() {
	const [open, setOpen] = useState(false);
	// Switching chats no longer blocks on a live turn: runs are keyed by run id and keep streaming
	// (and finalizing into IndexedDB) in whatever conversation they belong to, reappearing in place
	// when the user comes back.
	const activeConversationId = useGlobalChatStore(
		(s) => s.activeConversationId,
	);
	const newConversation = useGlobalChatStore((s) => s.newConversation);

	const sessions = useLiveQuery(
		() =>
			globalChatDb.sessions.orderBy("updatedAt").reverse().limit(50).toArray(),
		[],
	);

	// Shared restore path: settles stale mid-stream checkpoints (no eternal spinners) and
	// re-attaches runs of this conversation that are still streaming in Rust.
	const handleResume = useCallback(async (sessionId: string) => {
		await restoreGlobalChatConversation(sessionId);
		setOpen(false);
	}, []);

	const handleDelete = useCallback(
		async (sessionId: string) => {
			await globalChatDb.messages.where("sessionId").equals(sessionId).delete();
			await globalChatDb.sessions.delete(sessionId);
			if (sessionId === useGlobalChatStore.getState().activeConversationId) {
				newConversation();
			}
		},
		[newConversation],
	);

	return (
		<div className="flex items-center gap-0.5 ml-auto shrink-0">
			<Tooltip>
				<TooltipTrigger asChild>
					<Button
						variant="ghost"
						size="icon"
						className="h-9 w-9 md:h-8 md:w-8 rounded-lg"
						aria-label="New chat"
						onClick={newConversation}
					>
						<SquarePenIcon className="size-4" />
					</Button>
				</TooltipTrigger>
				<TooltipContent side="bottom" className="text-xs">
					New chat
				</TooltipContent>
			</Tooltip>

			<Popover open={open} onOpenChange={setOpen}>
				<Tooltip>
					<TooltipTrigger asChild>
						<PopoverTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className="h-9 w-9 md:h-8 md:w-8 rounded-lg"
								aria-label="Chat history"
							>
								<HistoryIcon className="size-4" />
							</Button>
						</PopoverTrigger>
					</TooltipTrigger>
					<TooltipContent side="bottom" className="text-xs">
						History
					</TooltipContent>
				</Tooltip>
				<PopoverContent align="end" className="w-80 p-1.5 z-[10000]">
					<p className="px-2 py-1.5 text-xs font-medium text-muted-foreground">
						Conversations
					</p>
					<div className="max-h-80 overflow-y-auto">
						{(sessions?.filter((s) => s.appId === GLOBAL_CHAT_APP_ID) ?? [])
							.length === 0 && (
							<p className="px-2 py-6 text-center text-sm text-muted-foreground">
								No conversations yet.
							</p>
						)}
						{sessions
							?.filter((session) => session.appId === GLOBAL_CHAT_APP_ID)
							.map((session) => {
								const active = session.id === activeConversationId;
								return (
									<div
										key={session.id}
										className={`group flex items-center gap-1 rounded-lg ${active ? "bg-primary/10" : "hover:bg-muted/60"}`}
									>
										<button
											type="button"
											className="flex-1 min-w-0 px-2 py-2 text-left"
											onClick={() => void handleResume(session.id)}
										>
											<span className="block text-sm truncate">
												{session.summarization || "Untitled conversation"}
											</span>
											<span className="block text-[11px] text-muted-foreground">
												{relativeTime(session.updatedAt)}
											</span>
										</button>
										<Button
											variant="ghost"
											size="icon"
											className="h-7 w-7 mr-1 rounded-md opacity-0 group-hover:opacity-100 text-muted-foreground hover:text-destructive shrink-0"
											aria-label="Delete conversation"
											onClick={() => void handleDelete(session.id)}
										>
											<Trash2Icon className="size-3.5" />
										</Button>
									</div>
								);
							})}
					</div>
				</PopoverContent>
			</Popover>
		</div>
	);
}
