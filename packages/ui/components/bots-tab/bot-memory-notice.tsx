"use client";

import { useTranslation } from "@flow-like/locales";
import { BrainIcon, Loader2Icon } from "lucide-react";
import type { ReactNode } from "react";
import { toast } from "sonner";
import { useUpdateBot } from "../../hooks/use-bots";
import {
	type IBot,
	MEMORY_CAPABILITY,
} from "../../state/backend-state/bot-state";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { MemoryModelPicker } from "../global-chat/memory-model-picker";
import { Button } from "../ui/button";
import { hasFlowPilot } from "./editor/bot-draft";

export type BotMemoryOff = "capability" | "model";

/**
 * Why a bot saves and recalls nothing, if it doesn't: the bot lacks the `memory` capability
 * (FlowPilot bots always have it), or neither it nor the chat names an embedding model. A bot
 * without a row (the Assistant) has no capability to switch.
 */
export function useBotMemoryOff(
	bot: Pick<IBot, "system" | "capabilities" | "embedding_model"> | undefined,
	embeddingModel?: string | null,
): BotMemoryOff | null {
	const chatEmbeddingModel = useGlobalChatStore((s) => s.embeddingModelId);
	if (
		bot &&
		!hasFlowPilot(bot) &&
		!bot.capabilities.includes(MEMORY_CAPABILITY)
	)
		return "capability";
	return embeddingModel || chatEmbeddingModel ? null : "model";
}

function NoticeFrame({
	text,
	children,
}: Readonly<{ text: string; children: ReactNode }>) {
	return (
		<output
			data-slot="bot-memory-notice"
			className="flex shrink-0 flex-col items-start gap-2 border-b border-border bg-muted/40 px-4 py-3"
		>
			<p className="flex items-start gap-2 text-xs text-foreground">
				<BrainIcon
					className="mt-0.5 size-3.5 shrink-0 text-muted-foreground"
					aria-hidden
				/>
				{text}
			</p>
			{children}
		</output>
	);
}

function TurnOnMemory({ bot }: Readonly<{ bot: IBot }>) {
	const { t } = useTranslation("bots");
	const { mutate, isPending } = useUpdateBot();
	const turnOn = () =>
		mutate(
			{
				botId: bot.id,
				request: { capabilities: [...bot.capabilities, MEMORY_CAPABILITY] },
			},
			{
				onError: (cause) => {
					console.warn("Turning on the bot's memory failed", cause);
					toast.error(
						t("memory.turnOnFailed", "Memory could not be turned on"),
					);
				},
			},
		);
	return (
		<Button
			type="button"
			size="sm"
			variant="outline"
			disabled={isPending}
			onClick={turnOn}
		>
			{isPending && <Loader2Icon className="animate-spin" aria-hidden />}
			{t("memory.turnOn", "Turn on memory")}
		</Button>
	);
}

/** Says memory is off for the bot and offers the fix: the capability first, then the chat's memory model. */
export function BotMemoryNotice({
	reason,
	bot,
	botId,
	name,
}: Readonly<{
	reason: BotMemoryOff;
	bot?: IBot;
	botId: string | null;
	name: string;
}>) {
	const { t } = useTranslation("bots");
	if (reason === "capability" && bot) {
		return (
			<NoticeFrame
				text={t(
					"memory.noticeCapability",
					"Memory is switched off for {{name}}: nothing you say is saved or recalled.",
					{ name },
				)}
			>
				<TurnOnMemory bot={bot} />
			</NoticeFrame>
		);
	}
	return (
		<NoticeFrame
			text={t(
				"memory.noticeModel",
				"{{name}} can't remember anything yet: no memory model is selected.",
				{ name },
			)}
		>
			<div className="w-full max-w-xs">
				<MemoryModelPicker botId={botId} />
			</div>
		</NoticeFrame>
	);
}
