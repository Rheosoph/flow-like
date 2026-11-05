"use client";

import { useTranslation } from "@flow-like/locales";
import { CpuIcon } from "lucide-react";
import { BotAvatar } from "../bots/bot-avatar";
import { useChatBotReaction } from "./chat-bot-reaction";
import type { GlobalChatBotView } from "./use-global-chat-bot";

/** The model a bot's settings pin (Flux's too), or null where the chat's own picker chooses. */
export function pinnedBotModel(chatBot: GlobalChatBotView): string | null {
	return chatBot.bot?.model.primary || null;
}

/** The chat's own model, shown for a bot that pins none, so its chat names who answers. */
export function DefaultModelChip({
	botName,
	label,
}: Readonly<{ botName: string; label: string }>) {
	const { t } = useTranslation("bots");
	const hint = t(
		"chat.defaultModelHint",
		"{{name}} has no model of its own, so it uses the chat's default model",
		{ name: botName },
	);
	return (
		<span
			title={hint}
			className="inline-flex h-11 min-w-0 shrink items-center gap-1 rounded-lg px-2 text-xs text-muted-foreground sm:h-8"
		>
			<CpuIcon className="size-4 shrink-0" aria-hidden />
			<span className="hidden max-w-32 truncate sm:inline">{label}</span>
			<span className="sr-only">{hint}</span>
		</span>
	);
}

/** The bot's face as the empty chat's mark; it mirrors the pill's reaction. */
export function BotWelcomeMark({
	view,
}: Readonly<{ view: GlobalChatBotView }>) {
	const reaction = useChatBotReaction(view.botId);
	return (
		<span className="relative flex h-32 w-40 items-end justify-center">
			<span
				aria-hidden
				className="absolute bottom-0.5 left-1/2 h-3 w-20 -translate-x-1/2 rounded-full bg-muted blur-sm"
			/>
			<BotAvatar
				appearance={view.appearance}
				expression={reaction.expression}
				expressionOverridesState={reaction.expressionOverridesState}
				agentState={reaction.agentState}
				size={120}
				decorative
				className="relative"
			/>
		</span>
	);
}
