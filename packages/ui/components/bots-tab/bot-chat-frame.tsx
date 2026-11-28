"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type ReactNode,
	useId,
	useLayoutEffect,
	useRef,
	useState,
} from "react";
import { emitGuideEvent } from "../../lib/guide/guide-events";
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
import {
	BOT_PANEL_WIDTH,
	MIN_CHAT_WIDTH,
	useDockPanelLayout,
} from "./crew-split-layout";
import { PaneResizeHandle, useStoredWidth } from "./pane-resize";
import { SwipeBackLayer } from "./swipe-back-layer";

/** The open tab of a bot's panel (`null` while it is closed) and how to change it. */
export interface BotPanelControl {
	tab: BotPanelTab | null;
	onTabChange: (tab: BotPanelTab | null) => void;
}

const PANEL_WIDTH_KEY = "flow-like:bots:panel-width";
const MIN_PANEL_WIDTH = 320;
const MAX_PANEL_WIDTH = 720;

function useElementWidth(element: HTMLElement | null) {
	const [width, setWidth] = useState(0);
	useLayoutEffect(() => {
		if (!element) return;
		const measure = () => setWidth(element.getBoundingClientRect().width);
		measure();
		if (typeof ResizeObserver === "undefined") return;
		const observer = new ResizeObserver(measure);
		observer.observe(element);
		return () => observer.disconnect();
	}, [element]);
	return width;
}

/** The panel beside the chat, as wide as the reader dragged it while the chat keeps its minimum. */
function PanelColumn({
	frame,
	panelId,
	children,
}: Readonly<{
	frame: HTMLElement | null;
	panelId: string;
	children: ReactNode;
}>) {
	const { t } = useTranslation("bots");
	const column = useRef<HTMLDivElement>(null);
	const frameWidth = useElementWidth(frame);
	const stored = useStoredWidth(
		PANEL_WIDTH_KEY,
		BOT_PANEL_WIDTH,
		MIN_PANEL_WIDTH,
		MAX_PANEL_WIDTH,
	);
	const max = frameWidth
		? Math.min(MAX_PANEL_WIDTH, frameWidth - MIN_CHAT_WIDTH)
		: MAX_PANEL_WIDTH;
	const width = Math.max(MIN_PANEL_WIDTH, Math.min(stored.width, max));
	return (
		<div
			ref={column}
			className="relative flex min-h-0 shrink-0 flex-col border-l"
			style={{ width }}
		>
			<PaneResizeHandle
				edge="left"
				label={t("panel.resize", "Resize details")}
				valueText={t("panel.width", "Details width: {{width}} pixels", {
					width: Math.round(width),
				})}
				controls={panelId}
				width={width}
				min={MIN_PANEL_WIDTH}
				max={max}
				initial={BOT_PANEL_WIDTH}
				measure={() => column.current?.getBoundingClientRect().width ?? width}
				onResize={stored.resize}
				onCommit={stored.save}
				className="-left-1.5"
			/>
			{children}
		</div>
	);
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
	const [frame, setFrame] = useState<HTMLDivElement | null>(null);
	const tabs = botPanelTabs(view);
	const tab = shownBotPanelTab(panel.tab, tabs);
	const covered = tab !== null && layout === "sheet";
	const closePanel = () => panel.onTabChange(null);
	const overview = (onClose: () => void) =>
		tab && (
			<BotOverviewPanel
				id={panelId}
				view={view}
				layout={layout}
				tabs={{ list: tabs, shown: tab, onChange: panel.onTabChange }}
				actions={actions}
				trailing={covered ? end : undefined}
				onClose={onClose}
			/>
		);
	return (
		<div ref={setFrame} className="relative flex min-h-0 min-w-0 flex-1">
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
							onToggle={() => {
								if (tab === null) emitGuideEvent("bots:details-opened");
								panel.onTabChange(tab === null ? DEFAULT_BOT_PANEL_TAB : null);
							}}
						/>
					}
				/>
			</div>
			{tab && !covered && (
				<PanelColumn frame={frame} panelId={panelId}>
					{overview(closePanel)}
				</PanelColumn>
			)}
			{covered && (
				<SwipeBackLayer onBack={closePanel}>{overview}</SwipeBackLayer>
			)}
		</div>
	);
}
