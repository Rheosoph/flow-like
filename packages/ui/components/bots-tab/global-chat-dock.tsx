"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowLeftIcon, XIcon } from "lucide-react";
import { useLayoutEffect } from "react";
import { cn } from "../../lib/utils";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { switchGlobalChatBot } from "../../state/global-chat/global-chat-stream";
import { useDraftRelay } from "../global-chat/chat-bot-avatar";
import { GlobalChatBody } from "../global-chat/global-chat-body";
import { useChatBotView } from "../global-chat/use-global-chat-bot";
import { Button } from "../ui/button";
import { BotChatFrame, type BotPanelControl } from "./bot-chat-frame";
import type { BotPanelActions } from "./bot-details";

const CORNER_BUTTON = "size-8 rounded-full text-muted-foreground";

/**
 * A chat in the dock: Flux's, the Assistant's or a custom bot's, all answered through the global
 * chat. Opening it switches the chat to that bot's conversation before the next paint, and the chat
 * shows its loading state until that lands, so the previous bot's messages never show under the new
 * bot's name; the pill and the panel show the dock's bot from the start, so a panel never opens on
 * the previous bot's workspace.
 * Interacting here counts as using the full chat page. Below the dock breakpoint it fills the
 * view, so "back" replaces "close".
 */
export function GlobalChatDock({
	botId,
	panel,
	actions,
	onClose,
}: Readonly<{
	/** Null for Flux; `general` for the Assistant without an account row. */
	botId: string | null;
	panel: BotPanelControl;
	actions: BotPanelActions;
	onClose: () => void;
}>) {
	const { t } = useTranslation("bots");
	const enableOverlayAutoOpen = useGlobalChatStore(
		(s) => s.enableOverlayAutoOpen,
	);
	const draft = useDraftRelay();
	const chatBot = useChatBotView(botId);
	useLayoutEffect(() => {
		void switchGlobalChatBot(botId);
	}, [botId]);
	const back = t("crew.dock.back", "Back to your crew");
	const close = t("crew.dock.close", "Close chat");
	return (
		<BotChatFrame
			view={chatBot}
			relay={draft.listener}
			panel={panel}
			actions={actions}
			start={
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className={cn(CORNER_BUTTON, "hidden @max-[820px]/bots:inline-flex")}
					onClick={onClose}
					aria-label={back}
					title={back}
				>
					<ArrowLeftIcon aria-hidden />
				</Button>
			}
			end={
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className={cn(CORNER_BUTTON, "@max-[820px]/bots:hidden")}
					onClick={onClose}
					aria-label={close}
					title={close}
				>
					<XIcon aria-hidden />
				</Button>
			}
		>
			<div
				className="flex min-h-0 flex-1 flex-col"
				onPointerDownCapture={enableOverlayAutoOpen}
				onKeyDownCapture={enableOverlayAutoOpen}
			>
				<GlobalChatBody
					variant="overlay"
					botId={botId}
					onDraftChange={draft.report}
					pillInset
				/>
			</div>
		</BotChatFrame>
	);
}
