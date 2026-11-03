"use client";

import { useTranslation } from "@flow-like/locales";
import { MoreHorizontalIcon, SettingsIcon, ZapIcon } from "lucide-react";
import { cn } from "../../lib/utils";
import {
	selectAutoMode,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import { Button } from "../ui/button";
import {
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
} from "../ui/dialog";
import { Popover, PopoverContent, PopoverTrigger } from "../ui/popover";
import {
	BotModelChip,
	DefaultModelChip,
	pinnedBotModel,
} from "./bot-chat-parts";
import { ChatActivityMenu } from "./chat-activity";
import { ChatMemorySettings } from "./chat-memory-settings";
import { ChatModelSettings } from "./chat-model-settings";
import { GlobalChatHistory } from "./global-chat-history";
import type { GlobalChatBotView } from "./use-global-chat-bot";

/** The toggle is the open chat owner's: this bot's, or Flux's for every Flux chat. */
function AutoModeToggle() {
	const { t } = useTranslation("chat");
	const conversationId = useGlobalChatStore((s) => s.activeConversationId);
	const autoMode = useGlobalChatStore((s) => selectAutoMode(s, conversationId));
	const setAutoMode = useGlobalChatStore((s) => s.setAutoMode);
	return (
		<Button
			type="button"
			variant="ghost"
			size="sm"
			aria-pressed={autoMode}
			onClick={() => setAutoMode(!autoMode, conversationId)}
			title={
				autoMode
					? t(
							"autoModeOnToolsRunAndChangesApplyWithoutAskingIncludingDeletionsAndFullboardReplacements",
							"Auto mode on. Tools run and changes apply without asking, including deletions and full-board replacements.",
						)
					: t(
							"autoModeOffTheAssistantAsksBeforeActing",
							"Auto mode off. The assistant asks before acting.",
						)
			}
			className={cn(
				"h-11 sm:h-8 shrink-0 gap-1.5 rounded-lg px-2 text-xs outline-none focus-visible:ring-2 focus-visible:ring-primary/40 focus-visible:ring-offset-0",
				autoMode && "bg-primary/12 text-primary hover:bg-primary/18",
			)}
		>
			<ZapIcon className="size-3.5 shrink-0" />
			{t("auto", "Auto")}
		</Button>
	);
}

/**
 * What a bot's composer adds to the standard one: the model that answers (the bot's own, else the
 * chat's default), Auto, and a menu with updates, Flux's history and, for a bot without a row, its
 * settings.
 */
export function BotChatActions({
	chatBot,
	defaultModelLabel,
	onOpenSettings,
}: Readonly<{
	chatBot: GlobalChatBotView;
	/** The chat's default model, for a bot that pins none; null until it is known. */
	defaultModelLabel: string | null;
	onOpenSettings: () => void;
}>) {
	const { t } = useTranslation("chat");
	const pinnedModel = pinnedBotModel(chatBot);
	const options = t("conversationOptions", "Conversation options");
	return (
		<>
			{pinnedModel ? (
				<BotModelChip botName={chatBot.name} modelId={pinnedModel} />
			) : (
				defaultModelLabel && (
					<DefaultModelChip botName={chatBot.name} label={defaultModelLabel} />
				)
			)}
			<AutoModeToggle />
			<Popover>
				<PopoverTrigger asChild>
					<Button
						type="button"
						variant="ghost"
						size="icon"
						className="h-11 w-11 shrink-0 sm:h-8 sm:w-8"
						aria-label={options}
						title={options}
					>
						<MoreHorizontalIcon className="size-4" />
					</Button>
				</PopoverTrigger>
				<PopoverContent side="top" align="start" className="z-10000 w-auto p-2">
					<p className="px-2 pb-2 text-xs font-medium text-muted-foreground">
						{options}
					</p>
					<div className="flex items-center gap-1">
						{!chatBot.bot && (
							<Button
								type="button"
								variant="ghost"
								size="icon"
								onClick={onOpenSettings}
								aria-label={t("botSettings", "Bot settings")}
								title={t("botSettings", "Bot settings")}
							>
								<SettingsIcon className="size-4" />
							</Button>
						)}
						<ChatActivityMenu />
						<GlobalChatHistory />
					</div>
				</PopoverContent>
			</Popover>
		</>
	);
}

/** Model and memory for a bot that has no row of its own to edit. */
export function BotSettingsDialog({
	chatBot,
	open,
	onOpenChange,
}: Readonly<{
	chatBot: GlobalChatBotView;
	open: boolean;
	onOpenChange: (open: boolean) => void;
}>) {
	const { t } = useTranslation("chat");
	if (chatBot.bot) return null;
	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent className="max-w-md">
				<DialogHeader>
					<DialogTitle>{t("botSettings", "Bot settings")}</DialogTitle>
					<DialogDescription>{chatBot.name}</DialogDescription>
				</DialogHeader>
				<ChatModelSettings />
				<ChatMemorySettings botId={chatBot.botId} />
			</DialogContent>
		</Dialog>
	);
}
