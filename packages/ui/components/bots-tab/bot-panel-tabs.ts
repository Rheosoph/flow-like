import {
	type BotCapability,
	type BotSystemKey,
	type IBot,
	MEMORY_CAPABILITY,
	SYSTEM_BOT_CAPABILITIES,
} from "../../state/backend-state/bot-state";
import { ASSISTANT_BOT_ID } from "../../state/global-chat/global-chat-bots";
import { offersEmail } from "./bot-mail";
import { hasFlowPilot } from "./editor/bot-draft";

/** The tabs of a bot's overview panel, in order. `?panel=` names the open one. */
export const BOT_PANEL_TABS = [
	"details",
	"files",
	"data",
	"computer",
	"memory",
	"mail",
] as const;
export type BotPanelTab = (typeof BOT_PANEL_TABS)[number];
export const DEFAULT_BOT_PANEL_TAB: BotPanelTab = "details";

export function isBotPanelTab(value: string): value is BotPanelTab {
	return (BOT_PANEL_TABS as readonly string[]).includes(value);
}

/** The bot a chat talks to: `null` is Flux, `general` the Assistant without a row. */
export interface BotPanelSubject {
	botId: string | null;
	bot?: IBot;
}

/** Flux and a built-in bot kept only on this computer have no workspace yet. */
export function hasBotWorkspace(bot: Pick<IBot, "system" | "local">): boolean {
	return bot.system === null || (bot.system === "general" && !bot.local);
}

export function subjectSystem({
	botId,
	bot,
}: BotPanelSubject): BotSystemKey | null {
	if (bot) return bot.system;
	if (botId === null) return "flux";
	return botId === ASSISTANT_BOT_ID ? "general" : null;
}

/** A row's abilities; a built-in without one has its defaults, a bot that's gone has none. */
export function subjectCapabilities(
	subject: BotPanelSubject,
): readonly BotCapability[] {
	if (subject.bot) return subject.bot.capabilities;
	const system = subjectSystem(subject);
	return system ? SYSTEM_BOT_CAPABILITIES[system] : [];
}

/** FlowPilot bots always remember; every other bot while `memory` is on. */
export function subjectRemembers(subject: BotPanelSubject): boolean {
	const capabilities = subjectCapabilities(subject);
	return (
		capabilities.includes(MEMORY_CAPABILITY) ||
		hasFlowPilot({ system: subjectSystem(subject), capabilities })
	);
}

/** Whose memories the Memory tab lists: Flux remembers per profile, every other bot on its own. */
export function subjectMemoryBotId(subject: BotPanelSubject): string | null {
	if (subjectSystem(subject) === "flux") return null;
	return subject.bot?.id ?? subject.botId;
}

/** A bot that drives a browser or the computer has a screen to watch. */
function usesComputer(capabilities: readonly BotCapability[]): boolean {
	return (
		capabilities.includes("browser_use") ||
		capabilities.includes("computer_use")
	);
}

/** Details always; Files and Data need the ability and a workspace, Computer a browser or computer ability, Memory the ability. */
export function botPanelTabs(subject: BotPanelSubject): BotPanelTab[] {
	const capabilities = subjectCapabilities(subject);
	const workspace = subject.bot !== undefined && hasBotWorkspace(subject.bot);
	const tabs: BotPanelTab[] = ["details"];
	if (workspace && capabilities.includes("storage")) tabs.push("files");
	if (workspace && capabilities.includes("data_studio")) tabs.push("data");
	if (usesComputer(capabilities)) tabs.push("computer");
	if (subjectRemembers(subject)) tabs.push("memory");
	if (subject.bot && offersEmail(subject.bot)) tabs.push("mail");
	return tabs;
}

/** The tab on screen: the one asked for while the bot has it, else Details; `null` keeps it closed. */
export function shownBotPanelTab(
	requested: BotPanelTab | null,
	tabs: readonly BotPanelTab[],
): BotPanelTab | null {
	if (requested === null) return null;
	return tabs.includes(requested) ? requested : DEFAULT_BOT_PANEL_TAB;
}
