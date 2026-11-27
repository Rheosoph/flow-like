"use client";

import { useTranslation } from "@flow-like/locales";
import { ArrowLeftIcon, PencilIcon, XIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../../lib/utils";
import { BotAvatar } from "../bots/bot-avatar";
import { useChatBotReaction } from "../global-chat/chat-bot-reaction";
import type { GlobalChatBotView } from "../global-chat/use-global-chat-bot";
import { Button } from "../ui/button";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../ui/tabs";
import { BotCalendarPane } from "./bot-calendar";
import { BotComputerPane } from "./bot-computer-pane";
import { BotDetails, type BotPanelActions } from "./bot-details";
import { BotMailPane } from "./bot-mail";
import {
	type BotPanelTab,
	isBotPanelTab,
	isWorkspaceTab,
	subjectMemoryBotId,
} from "./bot-panel-tabs";
import { BotWorkspaceLink } from "./bot-workspace-link";
import {
	BotDataPane,
	BotFilesPane,
	BotMemoryPane,
} from "./bot-workspace-panes";
import type { DockPanelLayout } from "./crew-split-layout";

const ACTION_BUTTON = "h-8 gap-1.5 rounded-full px-3.5 text-[13px]";

function BotPanelPane({
	tab,
	view,
	actions,
	onTab,
}: Readonly<{
	tab: BotPanelTab;
	view: GlobalChatBotView;
	actions: BotPanelActions;
	onTab: (tab: BotPanelTab) => void;
}>) {
	const { bot, name } = view;
	if (tab === "calendar" && bot) {
		return (
			<BotCalendarPane
				key={bot.id}
				bot={bot}
				name={name}
				onEdit={actions.onEdit}
				onTab={onTab}
			/>
		);
	}
	if (tab === "memory") {
		return (
			<BotMemoryPane
				key={view.botId ?? "flux"}
				botId={subjectMemoryBotId(view)}
				bot={bot}
				embeddingModel={bot?.embedding_model}
				name={name}
			/>
		);
	}
	if (tab === "files" && bot)
		return <BotFilesPane key={bot.id} bot={bot} name={name} />;
	if (tab === "data" && bot)
		return <BotDataPane key={bot.id} bot={bot} name={name} />;
	if (tab === "mail" && bot) return <BotMailPane key={bot.id} bot={bot} />;
	if (tab === "computer" && bot)
		return <BotComputerPane key={bot.id} bot={bot} name={name} />;
	return <BotDetails view={view} actions={actions} />;
}

/** Changing the bot, and its workspace app where it keeps one. */
function PanelActions({
	view,
	tabs,
	actions,
}: Readonly<{
	view: GlobalChatBotView;
	tabs: BotPanelTabs;
	actions: BotPanelActions;
}>) {
	const { t } = useTranslation("bots");
	const { bot } = view;
	if (!bot) return null;
	const { onEdit } = actions;
	return (
		<div className="mt-1.5 flex flex-wrap items-center justify-center gap-2">
			{onEdit && (
				<Button
					type="button"
					variant="outline"
					size="sm"
					className={ACTION_BUTTON}
					onClick={() => onEdit(bot)}
				>
					<PencilIcon className="size-3.5" aria-hidden />
					{t("panel.editBot", "Edit bot")}
				</Button>
			)}
			{tabs.list.some(isWorkspaceTab) && (
				<BotWorkspaceLink
					key={bot.id}
					bot={bot}
					name={view.name}
					className={ACTION_BUTTON}
				/>
			)}
		</div>
	);
}

/** The tabs a bot has, the one on screen and how to switch. */
export interface BotPanelTabs {
	list: readonly BotPanelTab[];
	shown: BotPanelTab;
	onChange: (tab: BotPanelTab) => void;
}

/**
 * A bot's overview: its portrait, name and the tabs its abilities bring. Beside the chat it closes;
 * as a sheet over the chat it goes back to it and carries the surface's own buttons (`trailing`).
 */
export function BotOverviewPanel({
	id,
	view,
	layout,
	tabs,
	actions,
	trailing,
	onClose,
}: Readonly<{
	id: string;
	view: GlobalChatBotView;
	layout: DockPanelLayout;
	tabs: BotPanelTabs;
	actions: BotPanelActions;
	trailing?: ReactNode;
	onClose: () => void;
}>) {
	const { t } = useTranslation("bots");
	const reaction = useChatBotReaction(view.botId);
	const labels: Readonly<Record<BotPanelTab, string>> = {
		details: t("workspace.tabs.details", "Details"),
		files: t("workspace.tabs.files", "Files"),
		data: t("workspace.tabs.data", "Data"),
		computer: t("workspace.tabs.computer", "Computer"),
		memory: t("workspace.tabs.memory", "Memory"),
		calendar: t("workspace.tabs.calendar", "Calendar"),
		mail: t("workspace.tabs.mail", "Mail"),
	};
	const sheet = layout === "sheet";
	const closeLabel = sheet
		? t("panel.back", "Back to the chat")
		: t("panel.close", "Close details");
	return (
		<section
			id={id}
			aria-label={t("panel.label", "About {{name}}", { name: view.name })}
			className="flex h-full min-h-0 min-w-0 flex-col bg-background"
		>
			<div
				className={cn(
					"flex shrink-0 items-center gap-0.5 px-2 pt-2",
					!sheet && "flex-row-reverse",
				)}
			>
				<Button
					type="button"
					variant="ghost"
					size="icon"
					className="size-8 rounded-full text-muted-foreground"
					onClick={onClose}
					aria-label={closeLabel}
					title={closeLabel}
				>
					{sheet ? <ArrowLeftIcon aria-hidden /> : <XIcon aria-hidden />}
				</Button>
				{trailing && (
					<div className="ml-auto flex items-center gap-0.5">{trailing}</div>
				)}
			</div>
			<div className="-mt-6 flex shrink-0 flex-col items-center gap-1.5 px-6 pb-4 text-center">
				<span className="relative flex h-20 w-full items-end justify-center">
					<span
						aria-hidden
						className="absolute bottom-0.5 left-1/2 h-2.5 w-12 -translate-x-1/2 rounded-full bg-muted blur-sm"
					/>
					<BotAvatar
						appearance={view.appearance}
						expression={reaction.expression}
						expressionOverridesState={reaction.expressionOverridesState}
						agentState={reaction.agentState}
						size={76}
						decorative
						className="relative"
					/>
				</span>
				<h2 className="max-w-full truncate text-lg font-semibold tracking-tight">
					{view.name}
				</h2>
				<PanelActions view={view} tabs={tabs} actions={actions} />
			</div>
			<Tabs
				value={tabs.shown}
				onValueChange={(value) => {
					if (isBotPanelTab(value)) tabs.onChange(value);
				}}
				className="min-h-0 flex-1 gap-0"
			>
				<TabsList
					aria-label={t("panel.tabs", "{{name}}'s details", {
						name: view.name,
					})}
					className="h-auto w-full shrink-0 justify-start gap-1 rounded-none border-b bg-transparent px-3 py-0"
				>
					{tabs.list.map((item) => (
						<TabsTrigger
							key={item}
							value={item}
							className="relative h-10 flex-none rounded-none border-0 bg-transparent px-2.5 text-[13px] text-muted-foreground after:absolute after:inset-x-2 after:bottom-0 after:h-0.5 after:rounded-full hover:text-foreground data-[state=active]:bg-transparent data-[state=active]:text-foreground data-[state=active]:shadow-none data-[state=active]:after:bg-primary dark:text-muted-foreground dark:data-[state=active]:border-transparent dark:data-[state=active]:bg-transparent"
						>
							{labels[item]}
						</TabsTrigger>
					))}
				</TabsList>
				{/* A closing tab renders once more with the next bot; only the tab on screen may set up a workspace. */}
				{tabs.list.map((item) => (
					<TabsContent
						key={item}
						value={item}
						className="flex min-h-0 flex-col"
					>
						{item === tabs.shown && (
							<BotPanelPane
								tab={item}
								view={view}
								actions={actions}
								onTab={tabs.onChange}
							/>
						)}
					</TabsContent>
				))}
			</Tabs>
		</section>
	);
}
