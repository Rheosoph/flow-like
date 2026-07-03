"use client";

import { Button, type IEvent, useBackend } from "@flow-like/flow-like-ui";
import { ChatInterfaceMemoized } from "@flow-like/flow-like-ui/components/interfaces/chat-default";
import { parseUint8ArrayToJson } from "@flow-like/flow-like-ui/lib/uint8";
import { AnimatePresence, motion } from "framer-motion";
import { ChevronDownIcon, MessageSquareIcon, XIcon } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { InlineAppChat } from "../../lib/global-chat-store";

interface InlineAppChatCardProps {
	chat: InlineAppChat;
	onClose: (id: string) => void;
	/** Tighter height when rendered inside the docked overlay. */
	compact?: boolean;
}

/**
 * An app's chat event rendered inline inside the global FlowPilot view. Mounts the full simple-chat
 * surface (ChatInterfaceMemoized), so files, plan steps, interactions and streaming all work — the
 * user talks to the app without leaving the global conversation.
 */
export function InlineAppChatCard({
	chat,
	onClose,
	compact = false,
}: InlineAppChatCardProps) {
	const backend = useBackend();
	const [event, setEvent] = useState<IEvent | null>(null);
	const [error, setError] = useState<string | null>(null);
	const [expanded, setExpanded] = useState(true);
	const toolbarRef = useRef(null);
	const sidebarRef = useRef(null);

	useEffect(() => {
		let disposed = false;
		void (async () => {
			try {
				const events = await backend.eventState.getEvents(chat.appId);
				const found = events.find(
					(candidate) =>
						candidate.id === chat.eventId && candidate.event_type === "chat",
				);
				if (disposed) return;
				if (found) setEvent(found);
				else setError("This app chat is no longer available.");
			} catch {
				if (!disposed) setError("Failed to load the app chat.");
			}
		})();
		return () => {
			disposed = true;
		};
	}, [backend.eventState, chat.appId, chat.eventId]);

	// IEvent.config arrives as raw bytes — the chat config must be the parsed JSON.
	const config = useMemo(
		() => (event ? (parseUint8ArrayToJson(event.config) ?? {}) : {}),
		[event],
	);

	return (
		<motion.div
			layout
			initial={{ opacity: 0, y: 12, scale: 0.98 }}
			animate={{ opacity: 1, y: 0, scale: 1 }}
			exit={{ opacity: 0, y: 12, scale: 0.98 }}
			transition={{ type: "spring", stiffness: 380, damping: 32 }}
			className="mx-3 mb-2 rounded-xl border border-primary/25 bg-background/80 backdrop-blur-xl shadow-lg overflow-hidden shrink-0"
		>
			<div className="flex items-center justify-between gap-2 px-3 py-2 bg-primary/5 border-b border-primary/15">
				<button
					type="button"
					className="flex items-center gap-2 min-w-0 flex-1 text-left"
					onClick={() => setExpanded((open) => !open)}
					aria-expanded={expanded}
				>
					<span className="flex items-center justify-center size-6 rounded-md bg-primary/15 text-primary shrink-0">
						<MessageSquareIcon className="size-3.5" />
					</span>
					<span className="text-sm font-medium truncate">{chat.name}</span>
					<span className="ml-1 px-1.5 py-0.5 rounded-full bg-primary/10 text-primary text-[10px] font-medium tracking-wide shrink-0">
						App Chat
					</span>
					<ChevronDownIcon
						className={`size-4 text-muted-foreground shrink-0 ml-auto transition-transform ${expanded ? "" : "-rotate-90"}`}
					/>
				</button>
				<Button
					variant="ghost"
					size="icon"
					className="h-7 w-7 rounded-full shrink-0"
					aria-label="Close app chat"
					onClick={() => onClose(chat.id)}
				>
					<XIcon className="size-3.5" />
				</Button>
			</div>

			<AnimatePresence initial={false}>
				{expanded && (
					<motion.div
						initial={{ height: 0, opacity: 0 }}
						animate={{ height: "auto", opacity: 1 }}
						exit={{ height: 0, opacity: 0 }}
						transition={{ duration: 0.2 }}
					>
						<div
							className={`${compact ? "h-[320px] max-h-[45vh]" : "h-[420px] max-h-[55vh]"} overflow-hidden flex flex-col`}
						>
							{event ? (
								<ChatInterfaceMemoized
									appId={chat.appId}
									event={event}
									config={config}
									toolbarRef={toolbarRef}
									sidebarRef={sidebarRef}
								/>
							) : (
								<div className="flex flex-1 items-center justify-center text-sm text-muted-foreground p-6">
									{error ?? "Loading app chat…"}
								</div>
							)}
						</div>
					</motion.div>
				)}
			</AnimatePresence>
		</motion.div>
	);
}
