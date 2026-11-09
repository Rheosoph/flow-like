"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowLeftIcon, XIcon } from "lucide-react";
import { cn } from "../../lib/utils";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { useDraftRelay } from "../global-chat/chat-bot-avatar";
import { GlobalChatBody } from "../global-chat/global-chat-body";
import { useChatBotView } from "../global-chat/use-global-chat-bot";
import { Button } from "../ui/button";
import { BotChatFrame, type BotPanelControl } from "./bot-chat-frame";
import type { BotPanelActions } from "./bot-details";
import { BotDirectCall } from "./bot-direct-call";
import { botPanelTabs, shownBotPanelTab } from "./bot-panel-tabs";
import { useDockPanelLayout } from "./crew-split-layout";
import type { DockSelection } from "./dock-selection";
import { useBotChatRead } from "./use-bot-chat-read";
import { useDockChatSession } from "./use-dock-chat-session";

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
	sourceSelection,
	panel,
	actions,
	onClose,
}: Readonly<{
	/** Null for Flux; `general` for the Assistant without an account row. */
	botId: string | null;
	sourceSelection?: DockSelection;
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
	useDockChatSession(botId, sourceSelection);
	const layout = useDockPanelLayout();
	const tab = shownBotPanelTab(panel.tab, botPanelTabs(chatBot));
	useBotChatRead(botId, tab === null || layout === "column");
	const back = t("crew.dock.back", "Back to your crew");
	const close = t("crew.dock.close", "Close chat");
	return (
		<BotDirectCall key={botId} bot={botId ? chatBot.bot : undefined}>
			{({ button, bar }) => (
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
							className={cn(
								CORNER_BUTTON,
								"hidden @max-[820px]/bots:inline-flex",
							)}
							onClick={onClose}
							aria-label={back}
							title={back}
						>
							<ArrowLeftIcon aria-hidden />
						</Button>
					}
					end={
						<>
							{button}
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
						</>
					}
				>
					<div
						className="flex min-h-0 flex-1 flex-col"
						onPointerDownCapture={enableOverlayAutoOpen}
						onKeyDownCapture={enableOverlayAutoOpen}
					>
						{bar && <div className="shrink-0 pt-14">{bar}</div>}
						<GlobalChatBody
							variant="overlay"
							botId={botId}
							onDraftChange={draft.report}
							pillInset={!bar}
						/>
					</div>
				</BotChatFrame>
			)}
		</BotDirectCall>
	);
}
