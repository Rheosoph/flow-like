"use client";

import { useTranslation } from "@flow-like/locales";
import {
	ArrowUpRightIcon,
	BellIcon,
	MessageSquareIcon,
	SparklesIcon,
	XIcon,
} from "lucide-react";
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
import { BOTS_ROUTE } from "../bots-tab/dock-selection";
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
					if (pathname === BOTS_ROUTE) {
						useGlobalChatStore.getState().requestReveal();
					} else {
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

function ChatActivityIcon({
	target,
}: Readonly<{ target: ChatActivityTarget }>) {
	return (
		<span className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
			{target.kind === "flowpilot" ? (
				<SparklesIcon className="size-4" />
			) : (
				<MessageSquareIcon className="size-4" />
			)}
		</span>
	);
}

function ChatActivityPreview({
	entry,
}: Readonly<{ entry: ChatActivityEntry }>) {
	const { t } = useTranslation("chat");
	return (
		<span className="mt-1 line-clamp-2 wrap-break-word text-xs leading-relaxed text-muted-foreground">
			{entry.preview || t("newReplyReady", "A new reply is ready.")}
		</span>
	);
}

function ChatActivityToast({
	entry,
	onOpen,
	onClose,
}: Readonly<{
	entry: ChatActivityEntry;
	onOpen: () => void;
	onClose: () => void;
}>) {
	const { t } = useTranslation("chat");
	return (
		<div className="relative flex w-full items-start gap-3 rounded-xl border border-border bg-popover p-3.5 pr-10 text-popover-foreground shadow-floating">
			<ChatActivityIcon target={entry.target} />
			<div className="flex min-w-0 flex-1 flex-col">
				<span className="truncate text-sm font-medium">{entry.title}</span>
				<ChatActivityPreview entry={entry} />
				<Button
					type="button"
					size="sm"
					variant="secondary"
					className="mt-3 h-7 self-start px-2.5 text-xs"
					onClick={onOpen}
				>
					{t("openConversation", "Open conversation")}
					<ArrowUpRightIcon className="size-3.5" />
				</Button>
			</div>
			<Button
				type="button"
				variant="ghost"
				size="icon"
				className="absolute right-2 top-2 size-7 text-muted-foreground"
				aria-label={t("close", "Close")}
				onClick={onClose}
			>
				<XIcon className="size-3.5" />
			</Button>
		</div>
	);
}

/** One listener per app shell, including while every chat surface is closed. */
export function ChatActivityProvider() {
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
			toast.custom(
				(id) => (
					<ChatActivityToast
						entry={entry}
						onOpen={() => void openActivity(entry)}
						onClose={() => toast.dismiss(id)}
					/>
				),
				{
					id: `chat-activity:${entry.id}`,
					duration: 8_000,
					// Sonner sizes only its own styled toasts; a custom one shrinks to its content.
					className: "w-(--width)",
				},
			);
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
	}, [openActivity]);

	return null;
}

export function ChatActivityBadge({
	className,
	extra = 0,
}: Readonly<{ className?: string; extra?: number }>) {
	const entries = useChatActivityStore((state) => state.entries.length);
	const count = entries + extra;
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
				className="z-10000 w-90 max-w-[calc(100vw-2rem)] overflow-hidden p-0 shadow-floating"
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
								<ChatActivityIcon target={entry.target} />
								<span className="flex min-w-0 flex-1 flex-col">
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
									<ChatActivityPreview entry={entry} />
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
