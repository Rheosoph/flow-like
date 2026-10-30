"use client";

import { useTranslation } from "@flow-like/locales";
import { BellIcon, MessageSquareIcon, SparklesIcon } from "lucide-react";
import { usePathname } from "next/navigation";
import { useCallback, useEffect, useState } from "react";
import { toast } from "sonner";
import { useClientRouter } from "../../lib/client-navigation";
import { cn } from "../../lib/utils";
import {
	type ChatActivityEntry,
	type ChatActivityTarget,
	initializeChatActivity,
	refreshChatActivityVisibility,
	registerChatActivitySurface,
	subscribeChatActivity,
	syncChatActivity,
	useChatActivityStore,
} from "../../state/chat-activity-store";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { Button } from "../ui/button";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";

/** Register the conversation actually rendered by a mounted chat surface. */
export function useChatActivityVisible(target: ChatActivityTarget | null) {
	const kind = target?.kind;
	const sessionId = target?.sessionId;
	const appId = target?.kind === "app" ? target.appId : undefined;
	const eventId = target?.kind === "app" ? target.eventId : undefined;
	useEffect(() => {
		if (!kind || !sessionId) return;
		if (kind === "flowpilot") {
			return registerChatActivitySurface({ kind, sessionId });
		}
		if (appId) {
			return registerChatActivitySurface({ kind, sessionId, appId, eventId });
		}
	}, [kind, sessionId, appId, eventId]);
}

function useOpenChatActivity() {
	const router = useClientRouter();
	const pathname = usePathname();
	const { t } = useTranslation("chat");
	return useCallback(
		async (entry: ChatActivityEntry) => {
			try {
				if (entry.target.kind === "flowpilot") {
					const { restoreGlobalChatConversation } = await import(
						"../../state/global-chat/global-chat-stream"
					);
					const restored = await restoreGlobalChatConversation(
						entry.target.sessionId,
					);
					if (!restored) return false;
					if (pathname !== "/chat") {
						useGlobalChatStore.getState().openOverlay();
					}
				} else {
					const params = new URLSearchParams({
						id: entry.target.appId,
						sessionId: entry.target.sessionId,
					});
					if (entry.target.eventId) params.set("eventId", entry.target.eventId);
					// An explicit switch to an app conversation should also prevent a live
					// FlowPilot run from reopening the dock on the route change.
					useGlobalChatStore.getState().dismissOverlay();
					router.push(`/use?${params}`);
				}
				// The destination clears unread activity once its conversation is visible.
				toast.dismiss(`chat-activity:${entry.id}`);
				return true;
			} catch {
				toast.error(
					t("couldNotOpenConversation", "Could not open this conversation."),
				);
				return false;
			}
		},
		[pathname, router, t],
	);
}

/** One listener per app shell, including while every chat surface is closed. */
export function ChatActivityProvider() {
	const { t } = useTranslation("chat");
	const openActivity = useOpenChatActivity();

	useEffect(() => {
		initializeChatActivity();
		const unsubscribeRead = useChatActivityStore.subscribe(
			(state, previous) => {
				if (state.entries === previous.entries) return;
				const unreadIds = new Set(state.entries.map((entry) => entry.id));
				for (const entry of previous.entries) {
					if (!unreadIds.has(entry.id))
						toast.dismiss(`chat-activity:${entry.id}`);
				}
			},
		);
		const unsubscribe = subscribeChatActivity((entry) => {
			toast(t("flowpilotActivity", "FlowPilot activity"), {
				id: `chat-activity:${entry.id}`,
				icon: <SparklesIcon className="size-4 text-primary" />,
				description: (
					<div className="space-y-1">
						<p className="truncate font-medium text-foreground">
							{entry.title}
						</p>
						<p className="line-clamp-2 break-words text-muted-foreground">
							{entry.preview || t("newReplyReady", "A new reply is ready.")}
						</p>
					</div>
				),
				action: {
					label: t("openConversation", "Open conversation"),
					onClick: () => void openActivity(entry),
				},
				closeButton: true,
				duration: 8_000,
			});
		});
		document.addEventListener(
			"visibilitychange",
			refreshChatActivityVisibility,
		);
		window.addEventListener("focus", refreshChatActivityVisibility);
		window.addEventListener("storage", syncChatActivity);
		refreshChatActivityVisibility();
		return () => {
			unsubscribe();
			unsubscribeRead();
			document.removeEventListener(
				"visibilitychange",
				refreshChatActivityVisibility,
			);
			window.removeEventListener("focus", refreshChatActivityVisibility);
			window.removeEventListener("storage", syncChatActivity);
		};
	}, [openActivity, t]);

	return null;
}

export function ChatActivityBadge({
	className,
}: Readonly<{ className?: string }>) {
	const count = useChatActivityStore((state) => state.entries.length);
	const { t } = useTranslation("chat");
	if (count === 0) return null;
	return (
		<output
			className={cn(
				"inline-flex h-4 min-w-4 shrink-0 items-center justify-center rounded-full bg-primary px-1 text-[9px] font-semibold leading-none text-primary-foreground ring-2 ring-background",
				className,
			)}
			aria-label={t(
				"unreadConversationUpdates",
				"{{count}} unread conversation updates",
				{ count },
			)}
		>
			{count > 99 ? "99+" : count}
		</output>
	);
}

export function ChatActivityMenu() {
	const { t, i18n } = useTranslation("chat");
	const entries = useChatActivityStore((state) => state.entries);
	const [open, setOpen] = useState(false);
	const [openingId, setOpeningId] = useState<string | null>(null);
	const openActivity = useOpenChatActivity();

	return (
		<Popover open={open} onOpenChange={setOpen}>
			<PopoverTrigger asChild>
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className="relative size-9 shrink-0 rounded-lg md:size-8"
					aria-label={t("conversationUpdates", "Conversation updates")}
					title={t("conversationUpdates", "Conversation updates")}
				>
					<BellIcon className="size-4" />
					<ChatActivityBadge className="absolute -right-0.5 -top-0.5" />
				</Button>
			</PopoverTrigger>
			<PopoverContent
				align="end"
				sideOffset={8}
				className="z-[10000] w-[360px] max-w-[calc(100vw-2rem)] overflow-hidden p-0 shadow-floating"
				aria-label={t("conversationUpdates", "Conversation updates")}
			>
				<div className="border-b border-border/50 px-4 py-3">
					<p className="text-sm font-semibold">
						{t("conversationUpdates", "Conversation updates")}
					</p>
					<p className="mt-0.5 text-xs text-muted-foreground">
						{t(
							"backgroundRepliesDescription",
							"Replies received while you were away.",
						)}
					</p>
				</div>
				{entries.length === 0 ? (
					<div className="flex flex-col items-center gap-2 px-4 py-8 text-center">
						<MessageSquareIcon className="size-5 text-muted-foreground/60" />
						<p className="text-sm text-muted-foreground">
							{t("noUnreadConversations", "You're all caught up.")}
						</p>
					</div>
				) : (
					<div className="max-h-[min(60vh,400px)] overflow-y-auto p-1.5">
						{entries.map((entry) => (
							<button
								key={entry.id}
								type="button"
								disabled={openingId !== null}
								onClick={async () => {
									setOpeningId(entry.id);
									if (await openActivity(entry)) setOpen(false);
									setOpeningId(null);
								}}
								className="flex w-full items-start gap-3 rounded-lg px-2.5 py-3 text-left transition-colors hover:bg-accent/70 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-primary/40 disabled:opacity-60"
							>
								<span className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
									{entry.target.kind === "flowpilot" ? (
										<SparklesIcon className="size-4" />
									) : (
										<MessageSquareIcon className="size-4" />
									)}
								</span>
								<span className="min-w-0 flex-1">
									<span className="flex items-center justify-between gap-2">
										<span className="truncate text-sm font-medium">
											{entry.title}
										</span>
										<time
											dateTime={new Date(entry.createdAt).toISOString()}
											className="shrink-0 text-[10px] text-muted-foreground"
										>
											{new Date(entry.createdAt).toLocaleTimeString(
												i18n.language,
												{ hour: "numeric", minute: "2-digit" },
											)}
										</time>
									</span>
									<span className="mt-1 line-clamp-2 break-words text-xs leading-relaxed text-muted-foreground">
										{entry.preview ||
											t("newReplyReady", "A new reply is ready.")}
									</span>
								</span>
								<span
									aria-hidden
									className="mt-2 size-1.5 shrink-0 rounded-full bg-primary"
								/>
							</button>
						))}
					</div>
				)}
			</PopoverContent>
		</Popover>
	);
}
