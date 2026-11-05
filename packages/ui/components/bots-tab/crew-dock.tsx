"use client";

import { useTranslation } from "@flow-like/locales";
import type { ReactNode } from "react";
import type { IBot } from "../../state/backend-state/bot-state";
import { ASSISTANT_BOT_ID } from "../../state/global-chat/global-chat-bots";
import { Button } from "../ui/button";
import { Skeleton } from "../ui/skeleton";
import type { BotPanelControl } from "./bot-chat-frame";
import type { BotPanelActions } from "./bot-details";
import { ConversationPanel } from "./conversation-panel";
import type { CrewMode } from "./crew-model";
import { DockHeader } from "./dock-header";
import {
	type DockSelection,
	chatBotIdOf,
	systemDockBot,
} from "./dock-selection";
import { GlobalChatDock } from "./global-chat-dock";

function DockMessage({
	title,
	body,
	action,
	loading = false,
	onClose,
}: Readonly<{
	title: string;
	body?: string;
	action?: ReactNode;
	loading?: boolean;
	onClose: () => void;
}>) {
	return (
		<>
			<DockHeader title={title} onClose={onClose} />
			<div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 text-center text-sm text-muted-foreground">
				{loading ? (
					<div className="flex w-full flex-col items-center gap-3">
						<Skeleton className="size-32 rounded-3xl" />
						<Skeleton className="h-5 w-40" />
						<Skeleton className="h-4 w-56" />
					</div>
				) : (
					body
				)}
				{action}
			</div>
		</>
	);
}

export interface DockAccount {
	me?: string | null;
	mode: CrewMode;
	bots?: readonly IBot[];
	botsLoading: boolean;
	/** The list couldn't be read; a bot's chat waits for it rather than guessing. */
	botsFailed?: boolean;
	retryBots?: () => void;
}

/** What every bot's chat in the dock shares: its panel, the panel's actions and the way out. */
interface ChatDockProps {
	sourceSelection?: DockSelection;
	panel: BotPanelControl;
	actions: BotPanelActions;
	onClose: () => void;
}

function LoadingDock({ onClose }: Readonly<{ onClose: () => void }>) {
	const { t } = useTranslation("bots");
	return (
		<DockMessage
			title={t("crew.dock.loading", "Loading…")}
			loading
			onClose={onClose}
		/>
	);
}

function FailedDock({
	onRetry,
	onClose,
}: Readonly<{ onRetry?: () => void; onClose: () => void }>) {
	const { t } = useTranslation("bots");
	return (
		<DockMessage
			title={t("crew.dock.loadFailedTitle", "Couldn't load your bots")}
			body={t("crew.dock.loadFailed", "Their chats open once the list loads.")}
			action={
				onRetry && (
					<Button type="button" variant="outline" size="sm" onClick={onRetry}>
						{t("crew.retry", "Try again")}
					</Button>
				)
			}
			onClose={onClose}
		/>
	);
}

/**
 * The Assistant's row names its chat once the list has it. Without an account or a row on this
 * computer it is `general`; a list that failed is waited for, never answered as `general`.
 */
function AssistantDock({
	account,
	...chat
}: Readonly<ChatDockProps & { account: DockAccount }>) {
	const bot = systemDockBot("general", account.bots);
	if (bot) return <GlobalChatDock botId={chatBotIdOf(bot)} {...chat} />;
	if (account.botsLoading) return <LoadingDock onClose={chat.onClose} />;
	if (account.botsFailed) {
		return <FailedDock onRetry={account.retryBots} onClose={chat.onClose} />;
	}
	return <GlobalChatDock botId={ASSISTANT_BOT_ID} {...chat} />;
}

function BotDock({
	botId,
	account,
	...chat
}: Readonly<ChatDockProps & { botId: string; account: DockAccount }>) {
	const { t } = useTranslation("bots");
	const bot = account.bots?.find((item) => item.id === botId);
	if (bot) return <GlobalChatDock botId={chatBotIdOf(bot)} {...chat} />;
	if (account.botsLoading) return <LoadingDock onClose={chat.onClose} />;
	if (account.botsFailed) {
		return <FailedDock onRetry={account.retryBots} onClose={chat.onClose} />;
	}
	return (
		<DockMessage
			title={t("crew.dock.botUnavailableTitle", "Bot unavailable")}
			body={
				account.mode === "guest"
					? t("crew.dock.signInForBots", "Sign in to see your bots.")
					: t(
							"crew.dock.botUnavailable",
							"This bot isn't in your crew. It may have been deleted.",
						)
			}
			onClose={chat.onClose}
		/>
	);
}

/** Whatever is selected: a bot's chat with its panel (Flux's too), or a conversation. */
export function CrewDock({
	selection,
	account,
	...chat
}: Readonly<
	ChatDockProps & { selection: DockSelection; account: DockAccount }
>) {
	const { t } = useTranslation("bots");

	if (selection.type === "system") {
		if (selection.system === "general") {
			return <AssistantDock account={account} {...chat} />;
		}
		return <GlobalChatDock botId={null} {...chat} />;
	}
	if (selection.type === "bot") {
		return <BotDock botId={selection.botId} account={account} {...chat} />;
	}
	if (account.mode === "guest") {
		return (
			<DockMessage
				title={t("crew.dock.unavailableTitle", "Conversation unavailable")}
				body={t(
					"crew.dock.signInForConversations",
					"Sign in to see your conversations.",
				)}
				onClose={chat.onClose}
			/>
		);
	}
	return (
		<ConversationPanel
			conversationId={selection.conversationId}
			me={account.me}
			bots={account.bots}
			onClose={chat.onClose}
		/>
	);
}
