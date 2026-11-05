"use client";

import { type ReactNode, useId } from "react";
import type { DraftRelay } from "../global-chat/chat-bot-avatar";
import type { GlobalChatBotView } from "../global-chat/use-global-chat-bot";
import type { BotPanelActions } from "./bot-details";
import { BotOverviewPanel } from "./bot-overview-panel";
import {
	type BotPanelTab,
	DEFAULT_BOT_PANEL_TAB,
	botPanelTabs,
	shownBotPanelTab,
} from "./bot-panel-tabs";
import { BotPill } from "./bot-pill";
import { BOT_PANEL_WIDTH, useDockPanelLayout } from "./crew-split-layout";

/** The open tab of a bot's panel (`null` while it is closed) and how to change it. */
export interface BotPanelControl {
	tab: BotPanelTab | null;
	onTabChange: (tab: BotPanelTab | null) => void;
}

/** Floats over the top of the chat: the pill in the middle, the surface's own buttons at the sides. */
function ChatTopBand({
	start,
	pill,
	end,
}: Readonly<{ start?: ReactNode; pill: ReactNode; end?: ReactNode }>) {
	return (
		<div className="pointer-events-none absolute inset-x-0 top-0 z-20 grid h-14 grid-cols-[1fr_auto_1fr] items-start gap-2 bg-linear-to-b from-background via-background/75 to-transparent px-2 pt-2">
			<div className="flex min-w-0 justify-start *:pointer-events-auto">
				{start}
			</div>
			<div className="flex min-w-0 justify-center *:pointer-events-auto">
				{pill}
			</div>
			<div className="flex min-w-0 justify-end gap-0.5 *:pointer-events-auto">
				{end}
			</div>
		</div>
	);
}

/**
 * A bot's chat as the main surface, its pill floating at the top. The pill opens the bot's panel:
 * beside the chat where the dock has room, else as a sheet over it that keeps `end` in reach. The
 * chat stays mounted throughout, so a draft or a running reply survives a look at the files.
 */
export function BotChatFrame({
	view,
	relay,
	panel,
	actions,
	start,
	end,
	children,
}: Readonly<{
	view: GlobalChatBotView;
	relay: DraftRelay;
	panel: BotPanelControl;
	actions: BotPanelActions;
	start?: ReactNode;
	end?: ReactNode;
	/** The chat, with room left at its top for the pill. */
	children: ReactNode;
}>) {
	const layout = useDockPanelLayout();
	const panelId = useId();
	const tabs = botPanelTabs(view);
	const tab = shownBotPanelTab(panel.tab, tabs);
	const covered = tab !== null && layout === "sheet";
	const overview = tab && (
		<BotOverviewPanel
			id={panelId}
			view={view}
			layout={layout}
			tabs={{ list: tabs, shown: tab, onChange: panel.onTabChange }}
			actions={actions}
			trailing={covered ? end : undefined}
			onClose={() => panel.onTabChange(null)}
		/>
	);
	return (
		<div className="relative flex min-h-0 min-w-0 flex-1">
			<div
				className="relative flex min-h-0 min-w-0 flex-1 flex-col"
				inert={covered || undefined}
			>
				{children}
				<ChatTopBand
					start={start}
					end={end}
					pill={
						<BotPill
							view={view}
							relay={relay}
							expanded={tab !== null}
							controls={panelId}
							onToggle={() =>
								panel.onTabChange(tab === null ? DEFAULT_BOT_PANEL_TAB : null)
							}
						/>
					}
				/>
			</div>
			{overview && !covered && (
				<div
					className="flex min-h-0 shrink-0 flex-col border-l"
					style={{ width: BOT_PANEL_WIDTH }}
				>
					{overview}
				</div>
			)}
			{covered && (
				<div className="absolute inset-0 z-30 flex flex-col animate-in fade-in slide-in-from-right-4 duration-200">
					{overview}
				</div>
			)}
		</div>
	);
}
