"use client";

import { AnimatePresence, motion } from "framer-motion";
import {
	ClockIcon,
	LayoutGridIcon,
	MessageSquareIcon,
	WorkflowIcon,
} from "lucide-react";
import { memo, useCallback, useEffect, useMemo, useState } from "react";

import { Button } from "../ui/button";

import { useIsMobile } from "../../hooks/use-mobile";
import {
	type IFlowPilotConversation,
	deleteConversation,
	getRecentConversations,
	renameConversation,
	setConversationPinned,
} from "../../lib/flowpilot-db";
import { ChatHistoryList } from "../chat-history/chat-history-list";
import type { IHistoryEntry } from "../chat-history/chat-history-types";
import type { AgentMode } from "./types";

interface HistoryPanelProps {
	mode: AgentMode;
	currentConversationId?: string;
	onSelectConversation: (conversation: IFlowPilotConversation) => void;
	onNewConversation: () => void;
	isOpen: boolean;
	onClose: () => void;
}

const MODE_ICONS = {
	board: WorkflowIcon,
	ui: LayoutGridIcon,
	both: MessageSquareIcon,
} as const;

export const HistoryPanel = memo(function HistoryPanel({
	mode,
	currentConversationId,
	onSelectConversation,
	onNewConversation,
	isOpen,
	onClose,
}: HistoryPanelProps) {
	const isMobile = useIsMobile();
	const [conversations, setConversations] = useState<
		IFlowPilotConversation[] | undefined
	>(undefined);

	const loadConversations = useCallback(async () => {
		try {
			setConversations(await getRecentConversations(50, mode));
		} catch (error) {
			console.error("Failed to load conversations:", error);
			setConversations([]);
		}
	}, [mode]);

	useEffect(() => {
		if (isOpen) void loadConversations();
	}, [isOpen, loadConversations]);

	// FlowPilot's conversation store keeps ISO strings; the shared list works in epoch millis.
	const entries = useMemo<IHistoryEntry[] | undefined>(
		() =>
			conversations?.map((conversation) => ({
				id: conversation.id,
				title: conversation.title,
				updatedAt: new Date(conversation.updatedAt).getTime(),
				pinnedAt: conversation.pinnedAt
					? new Date(conversation.pinnedAt).getTime()
					: undefined,
				subtitle: `${conversation.messageCount} message${
					conversation.messageCount === 1 ? "" : "s"
				}`,
				icon: MODE_ICONS[conversation.mode] ?? MessageSquareIcon,
			})),
		[conversations],
	);

	const handleSelect = useCallback(
		(id: string) => {
			const conversation = conversations?.find((entry) => entry.id === id);
			if (!conversation) return;
			onSelectConversation(conversation);
			onClose();
		},
		[conversations, onSelectConversation, onClose],
	);

	const handleNew = useCallback(() => {
		onNewConversation();
		onClose();
	}, [onNewConversation, onClose]);

	const handleTogglePin = useCallback(
		async (id: string, pinned: boolean) => {
			await setConversationPinned(id, pinned);
			await loadConversations();
		},
		[loadConversations],
	);

	const handleRename = useCallback(
		async (id: string, title: string) => {
			await renameConversation(id, title);
			await loadConversations();
		},
		[loadConversations],
	);

	const handleDelete = useCallback(async (id: string) => {
		try {
			await deleteConversation(id);
			setConversations((prev) => prev?.filter((c) => c.id !== id));
		} catch (error) {
			console.error("Failed to delete conversation:", error);
		}
	}, []);

	if (!isOpen) return null;

	return (
		<AnimatePresence>
			<motion.div
				initial={{ opacity: 0, x: -10 }}
				animate={{ opacity: 1, x: 0 }}
				exit={{ opacity: 0, x: -10 }}
				className="absolute inset-0 z-10 flex flex-col bg-background/95 backdrop-blur-sm"
			>
				<div className="flex shrink-0 items-center justify-between border-b px-3 py-2">
					<div className="flex items-center gap-2">
						<ClockIcon className="h-4 w-4 text-muted-foreground" />
						<span className="text-sm font-medium">History</span>
					</div>
					<Button
						size="sm"
						variant="ghost"
						className="h-7 px-2 text-xs"
						onClick={onClose}
					>
						Close
					</Button>
				</div>

				<ChatHistoryList
					className="min-h-0 flex-1"
					entries={entries}
					activeId={currentConversationId}
					onSelect={handleSelect}
					onNew={handleNew}
					onTogglePin={handleTogglePin}
					onRename={handleRename}
					onDelete={handleDelete}
					density={isMobile ? "comfortable" : "compact"}
					emptyDescription="Start a new chat to see it here."
				/>
			</motion.div>
		</AnimatePresence>
	);
});
