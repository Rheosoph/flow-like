"use client";

import { useTranslation } from "@flow-like/locales";
import { BrainIcon } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import { useAuth } from "react-oidc-context";
import { useInvoke } from "../../hooks/use-invoke";
import { asArray } from "../../lib/response-shape";
import { IBitTypes } from "../../lib/schema/bit/bit";
import { useBackend } from "../../state/backend-state";
import type { IBot } from "../../state/backend-state/bot-state";
import { ASSISTANT_BOT_ID } from "../../state/global-chat/global-chat-bots";
import {
	type MemoryOwner,
	clearGlobalChatMemory,
	globalChatMemoryStatus,
} from "../../state/global-chat/global-chat-memory";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
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
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import {
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui/select";
import { MemoryList } from "./memory-list";
import { useChatBotList } from "./use-global-chat-bot";

const MEMORY_OFF = "__off__";

/**
 * Every memory the one embedding model reads: Flux's in the profile (a Flux row's too), the
 * Assistant's without a row (`general`) and each listed bot's, those on this computer included.
 */
function sharedModelOwners(
	profileId: string,
	bots: readonly Pick<IBot, "id" | "system">[] | undefined,
	open: MemoryOwner,
): MemoryOwner[] {
	const botIds = new Set([ASSISTANT_BOT_ID]);
	for (const bot of bots ?? []) if (bot.system !== "flux") botIds.add(bot.id);
	if (open.botId) botIds.add(open.botId);
	return [
		{ profileId, botId: null },
		...[...botIds].map((botId) => ({ profileId, botId })),
	];
}

interface StaleMemories {
	owners: MemoryOwner[];
	count: number;
}

/** The owners holding memories another model embedded, and how many together; unreadable ones are skipped. */
async function memoriesFromOtherModels(
	owners: readonly MemoryOwner[],
	modelId: string,
	token?: string,
): Promise<StaleMemories> {
	const statusOf = (owner: MemoryOwner) => globalChatMemoryStatus(owner, token);
	const statuses = await Promise.allSettled(owners.map(statusOf));
	const stale: StaleMemories = { owners: [], count: 0 };
	for (const [index, status] of statuses.entries()) {
		if (
			status.status === "fulfilled" &&
			status.value.count > 0 &&
			status.value.embedding_model_id !== modelId
		) {
			stale.owners.push(owners[index]);
			stale.count += status.value.count;
		}
	}
	return stale;
}

/** Memory selection and saved entries belong to bot settings. */
export function ChatMemorySettings({
	botId,
}: Readonly<{ botId: string | null }>) {
	const { t } = useTranslation("chat");
	const auth = useAuth();
	const backend = useBackend();
	const embeddingModelId = useGlobalChatStore((s) => s.embeddingModelId);
	const setEmbeddingModelId = useGlobalChatStore((s) => s.setEmbeddingModelId);
	const settingsProfile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
		true,
	);
	const profileBits = useInvoke(
		backend.bitState.getProfileBits,
		backend.bitState,
		[],
		!!settingsProfile.data,
		[settingsProfile.data?.hub_profile.id],
	);
	const memoryModels = useMemo(
		() =>
			asArray(profileBits.data).filter(
				(bit) => bit.type === IBitTypes.Embedding,
			),
		[profileBits.data],
	);
	const [pendingEmbedding, setPendingEmbedding] = useState<
		(StaleMemories & { modelId: string }) | null
	>(null);
	const [memoryManagerOpen, setMemoryManagerOpen] = useState(false);
	const profileId = settingsProfile.data?.hub_profile.id;
	// Flux remembers per profile; a bot's chat reads and manages that bot's own memory.
	const memoryOwner = useMemo<MemoryOwner | null>(
		() => (profileId ? { profileId, botId } : null),
		[profileId, botId],
	);
	const bots = useChatBotList().data;

	// Switching the embedding model makes existing (differently-embedded) memories unreadable, so
	// warn + delete on confirm. Flux and every bot share the model (until a bot's own one reaches its
	// runs), so every owner's memories are checked and the prompt counts them all.
	const handleEmbeddingChange = useCallback(
		async (value: string) => {
			const newModelId = value === MEMORY_OFF ? "" : value;
			if (newModelId === embeddingModelId) return;
			if (!newModelId) {
				setEmbeddingModelId("");
				return;
			}
			if (!memoryOwner) {
				setEmbeddingModelId(newModelId);
				return;
			}
			const stale = await memoriesFromOtherModels(
				sharedModelOwners(memoryOwner.profileId, bots, memoryOwner),
				newModelId,
				auth.user?.access_token,
			);
			if (stale.count > 0) {
				setPendingEmbedding({ ...stale, modelId: newModelId });
				return;
			}
			setEmbeddingModelId(newModelId);
		},
		[
			embeddingModelId,
			setEmbeddingModelId,
			memoryOwner,
			bots,
			auth.user?.access_token,
		],
	);

	const confirmEmbeddingChange = useCallback(async () => {
		if (!pendingEmbedding) return;
		// Best-effort: a store that can't be cleared now stays unreadable either way.
		await Promise.allSettled(
			pendingEmbedding.owners.map((owner) =>
				clearGlobalChatMemory(owner, auth.user?.access_token),
			),
		);
		setEmbeddingModelId(pendingEmbedding.modelId);
		setPendingEmbedding(null);
	}, [pendingEmbedding, setEmbeddingModelId, auth.user?.access_token]);

	// Memory recall + the memory tools run on EVERY backend (the Rust loop wires `memory` into the
	// Copilot/Codex/Claude-Code agents and the Bits loop alike), so the picker is shown wherever the
	// profile has an embedding model. not gated to the Bits backend.
	const memoryPicker = useMemo(
		() =>
			memoryModels.length > 0 ? (
				<Select
					value={embeddingModelId || MEMORY_OFF}
					onValueChange={handleEmbeddingChange}
				>
					<SelectTrigger
						className="w-full"
						aria-label={t("memoryEmbeddingModel", "Memory embedding model")}
					>
						<BrainIcon className="size-3.5 mr-1 text-muted-foreground shrink-0" />
						<SelectValue placeholder={t("memoryOff", "Memory: off")} />
					</SelectTrigger>
					<SelectContent className="z-10000">
						<SelectItem value={MEMORY_OFF} className="text-xs">
							{t("memoryOff", "Memory: off")}
						</SelectItem>
						{memoryModels.map((bit) => (
							<SelectItem key={bit.id} value={bit.id} className="text-xs">
								{bit.meta?.en?.name ?? bit.id}
							</SelectItem>
						))}
					</SelectContent>
				</Select>
			) : null,
		[memoryModels, embeddingModelId, handleEmbeddingChange, t],
	);

	return (
		<section className="flex flex-col gap-2 border-t border-border pt-5">
			<h3 className="text-sm font-medium">{t("memory", "Memory")}</h3>
			<p className="text-xs leading-relaxed text-muted-foreground">
				{t(
					"botMemoryHint",
					"The embedding model for memory in chats whose bot doesn't set its own. Each bot keeps its own saved memories. Changes apply immediately.",
				)}
			</p>
			{memoryPicker ?? (
				<p className="text-xs text-muted-foreground">
					{t(
						"memoryModelNeeded",
						"Add an embedding model to your profile to enable memory.",
					)}
				</p>
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
					active={!!embeddingModelId}
				/>
			)}

			<AlertDialog
				open={pendingEmbedding !== null}
				onOpenChange={(open) => !open && setPendingEmbedding(null)}
			>
				<AlertDialogContent>
					<AlertDialogHeader>
						<AlertDialogTitle>
							{t(
								"changeMemoryEmbeddingModel",
								"Change memory embedding model?",
							)}
						</AlertDialogTitle>
						<AlertDialogDescription>
							{t("sharedEmbeddingModelDeletesMemories", {
								defaultValue_one:
									"{{count}} saved memory was embedded with a different model. Flux and every bot use this model, and the new one can't read it, so switching permanently deletes it. Continue?",
								defaultValue_other:
									"{{count}} saved memories were embedded with a different model. Flux and every bot use this model, and the new one can't read them, so switching permanently deletes them. Continue?",
								count: pendingEmbedding?.count ?? 0,
							})}
						</AlertDialogDescription>
					</AlertDialogHeader>
					<AlertDialogFooter>
						<AlertDialogCancel>
							{t("keepCurrentModel", "Keep current model")}
						</AlertDialogCancel>
						<AlertDialogAction onClick={confirmEmbeddingChange}>
							{t("deleteAmpSwitch", "Delete & switch")}
						</AlertDialogAction>
					</AlertDialogFooter>
				</AlertDialogContent>
			</AlertDialog>
		</section>
	);
}

/** The open chat owner's memories (Flux's per profile, or the bot's own); they load on open. */
function MemoryManagerDialog({
	open,
	onOpenChange,
	owner,
	active,
}: Readonly<{
	open: boolean;
	onOpenChange: (open: boolean) => void;
	owner: MemoryOwner;
	active: boolean;
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
				<MemoryList
					owner={owner}
					active={active}
					listClassName="max-h-[50vh]"
				/>
			</DialogContent>
		</Dialog>
	);
}
