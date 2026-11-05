"use client";

import { useTranslation } from "@flow-like/locales";
import { BrainIcon } from "lucide-react";
import { type ReactNode, useMemo, useState } from "react";
import { useInvoke } from "../../hooks/use-invoke";
import { useBackend } from "../../state/backend-state";
import type { MemoryOwner } from "../../state/global-chat/global-chat-memory";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { MemoryList } from "./memory-list";
import { MemoryModelPicker } from "./memory-model-picker";

/** Memory selection and saved entries belong to bot settings. */
export function ChatMemorySettings({
	botId,
	showModelPicker = true,
	memoryEnabled,
	memoryContent,
}: Readonly<{
	botId: string | null;
	/** Bot editors keep their model selection in the draft until it is saved. */
	showModelPicker?: boolean;
	memoryEnabled?: boolean;
	/** Workspace-backed bots review the facts kept by their memory engine. */
	memoryContent?: ReactNode;
}>) {
	const { t } = useTranslation("chat");
	const backend = useBackend();
	const embeddingModelId = useGlobalChatStore((s) => s.embeddingModelId);
	const settingsProfile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
		true,
	);
	const [memoryManagerOpen, setMemoryManagerOpen] = useState(false);
	const profileId = settingsProfile.data?.hub_profile.id;
	// Flux remembers per profile; a bot's chat reads and manages that bot's own memory.
	const memoryOwner = useMemo<MemoryOwner | null>(
		() => (profileId ? { profileId, botId } : null),
		[profileId, botId],
	);

	return (
		<section className="flex flex-col gap-2 border-t border-border pt-5">
			<h3 className="text-sm font-medium">{t("memory", "Memory")}</h3>
			{showModelPicker && (
				<>
					<p className="text-xs leading-relaxed text-muted-foreground">
						{t(
							"botMemoryHint",
							"The embedding model for memory in chats whose bot doesn't set its own. Each bot keeps its own saved memories. Changes apply immediately.",
						)}
					</p>
					<MemoryModelPicker botId={botId} />
				</>
			)}
			{memoryOwner && (
				<Button
					type="button"
					variant="outline"
					size="sm"
					className="self-start"
					onClick={() => setMemoryManagerOpen(true)}
				>
					<BrainIcon className="size-3.5" />
					{t("reviewManageSavedMemories", "Review & manage saved memories")}
				</Button>
			)}
			{memoryOwner && (
				<MemoryManagerDialog
					open={memoryManagerOpen}
					onOpenChange={setMemoryManagerOpen}
					owner={memoryOwner}
					active={memoryEnabled ?? !!embeddingModelId}
				>
					{memoryContent}
				</MemoryManagerDialog>
			)}
		</section>
	);
}

/** The open chat owner's memories (Flux's per profile, or the bot's own); they load on open. */
function MemoryManagerDialog({
	open,
	onOpenChange,
	owner,
	active,
	children,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	owner: MemoryOwner;
	active: boolean;
	children?: ReactNode;
}>) {
	const { t } = useTranslation("chat");
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-lg">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<BrainIcon className="size-4 text-primary" />
						{t("savedMemories", "Saved memories")}
					</DialogTitle>
					<DialogDescription>
						{t(
							"botSavedMemoriesDescription",
							"Facts, preferences, and decisions saved for this bot. Delete anything it should forget.",
						)}
					</DialogDescription>
				</DialogHeader>
				{children ? (
					<div className="flex max-h-[50vh] flex-col overflow-y-auto">
						{children}
					</div>
				) : (
					<MemoryList
						owner={owner}
						active={active}
						listClassName="max-h-[50vh]"
					/>
				)}
			</DialogContent>
		</Dialog>
	);
}
