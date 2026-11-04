import type { BotSystemKey, IBot } from "../../state/backend-state/bot-state";
import type { IConversation } from "../../state/backend-state/inbox-state";
import { ASSISTANT_BOT_ID } from "../../state/global-chat/global-chat-bots";
import { type BotPanelTab, isBotPanelTab } from "./bot-panel-tabs";
import { directPartnerId, isSelfPrincipal } from "./crew-model";

/** The Bots tab. Its dock shows the global conversation, so the docked overlay stays closed there. */
export const BOTS_ROUTE = "/bots";

export type DockSelection =
	| { type: "system"; system: BotSystemKey }
	| { type: "bot"; botId: string }
	| { type: "conversation"; conversationId: string };

export const DEFAULT_DOCK_SELECTION: DockSelection = {
	type: "system",
	system: "general",
};

/** Data Studio's place in one workspace; it means nothing in another bot's. */
const WORKSPACE_VIEW_PARAMS = [
	"table",
	"view",
	"overlay",
	"scope",
	"forecast",
	"branch",
	"version",
	"tag",
	"page",
	"pageSize",
	"source",
] as const;

const SYSTEM_KEYS: readonly BotSystemKey[] = ["general", "flux"];

function isSystemKey(value: string): value is BotSystemKey {
	return (SYSTEM_KEYS as readonly string[]).includes(value);
}

interface SearchParamsLike {
	get(name: string): string | null;
}

/** `/bots` alone is the Assistant; `?bot=` takes a bot id or a built-in key, `?conversation=` an id. */
export function parseDockSelection(params: SearchParamsLike): DockSelection {
	const conversationId = params.get("conversation")?.trim();
	if (conversationId) return { type: "conversation", conversationId };
	const bot = params.get("bot")?.trim();
	if (!bot) return DEFAULT_DOCK_SELECTION;
	return isSystemKey(bot)
		? { type: "system", system: bot }
		: { type: "bot", botId: bot };
}

/** `?panel=` names the open tab of the bot's overview panel; without it the panel is closed. */
export function parseDockPanel(params: SearchParamsLike): BotPanelTab | null {
	const panel = params.get("panel")?.trim() ?? "";
	return isBotPanelTab(panel) ? panel : null;
}

/**
 * The query string for a selection, keeping every parameter the dock doesn't own. Another bot
 * or conversation opens with its panel closed, without the last workspace's Data Studio place.
 */
export function dockSelectionQuery(
	selection: DockSelection,
	current: string | URLSearchParams,
): string {
	const params = new URLSearchParams(current);
	if (
		dockSelectionKey(parseDockSelection(params)) !== dockSelectionKey(selection)
	) {
		params.delete("panel");
		params.delete("session");
		for (const name of WORKSPACE_VIEW_PARAMS) params.delete(name);
	}
	params.delete("bot");
	params.delete("conversation");
	if (selection.type === "conversation") {
		params.set("conversation", selection.conversationId);
	} else if (selection.type === "bot") {
		params.set("bot", selection.botId);
	} else if (selection.system !== "general") {
		params.set("bot", selection.system);
	}
	return params.toString();
}

/** The query string with the selected bot's panel on `panel` (closed for `null`), keeping the rest. */
export function dockPanelQuery(
	panel: BotPanelTab | null,
	current: string | URLSearchParams,
): string {
	const params = new URLSearchParams(current);
	if (panel === null) params.delete("panel");
	else params.set("panel", panel);
	return params.toString();
}

/**
 * The row a built-in's dock shows: the account's own, else the one on this computer. A second one
 * (chatted with on this computer before signing in) opens as a bot of its own.
 */
export function systemDockBot(
	system: BotSystemKey,
	bots: readonly IBot[] | undefined,
): IBot | undefined {
	return (
		bots?.find((bot) => bot.system === system && !bot.local) ??
		bots?.find((bot) => bot.system === system)
	);
}

/** `?move=local` opens the review of bots that live only on this computer. */
export function isMoveRequested(params: SearchParamsLike): boolean {
	return params.get("move") === "local";
}

/** The query string without the move request, keeping every other parameter. */
export function withoutMoveRequest(current: string | URLSearchParams): string {
	const params = new URLSearchParams(current);
	params.delete("move");
	return params.toString();
}

/** A built-in's dock opens by key or by its row's id; a second one of a kind opens by id. */
export function resolveDockSelection(
	selection: DockSelection,
	bots: readonly IBot[] | undefined,
	conversations?: readonly IConversation[],
	me?: string | null,
): DockSelection {
	if (selection.type === "conversation") {
		const bot = ownBotOfDirectConversation(
			conversations?.find((item) => item.id === selection.conversationId),
			bots,
			me,
		);
		return bot ? dockSelectionForBot(bot, bots) : selection;
	}
	if (selection.type !== "bot") return selection;
	const bot = bots?.find((item) => item.id === selection.botId);
	return bot ? dockSelectionForBot(bot, bots) : selection;
}

/** The owner's bot on the other end of a direct conversation, which opens as that bot's chat. */
function ownBotOfDirectConversation(
	conversation: IConversation | undefined,
	bots: readonly IBot[] | undefined,
	me?: string | null,
): IBot | undefined {
	if (!conversation) return undefined;
	const partner = directPartnerId(conversation, me);
	if (!partner) return undefined;
	return bots?.find(
		(bot) =>
			bot.id === partner && (bot.local || isSelfPrincipal(bot.owner_id, me)),
	);
}

export function dockSelectionForBot(
	bot: Pick<IBot, "id" | "system">,
	bots: readonly IBot[] | undefined,
): DockSelection {
	if (!bot.system) return { type: "bot", botId: bot.id };
	const shown = systemDockBot(bot.system, bots);
	return !shown || shown.id === bot.id
		? { type: "system", system: bot.system }
		: { type: "bot", botId: bot.id };
}

/** Flux's row answers in Flux's conversations; every other row in its own. */
export function chatBotIdOf(bot: Pick<IBot, "id" | "system">): string | null {
	return bot.system === "flux" ? null : bot.id;
}

/**
 * The global chat bot a selection's dock answers with: null for Flux, the Assistant's row once the
 * list holds one (else `general`), a bot's own id. Undefined for a conversation, which has none.
 */
export function chatBotIdForSelection(
	selection: DockSelection,
	bots: readonly IBot[] | undefined,
): string | null | undefined {
	if (selection.type === "conversation") return undefined;
	if (selection.type === "bot") {
		const bot = bots?.find((item) => item.id === selection.botId);
		return bot ? chatBotIdOf(bot) : selection.botId;
	}
	if (selection.system === "flux") return null;
	const assistant = systemDockBot("general", bots);
	return assistant ? chatBotIdOf(assistant) : ASSISTANT_BOT_ID;
}

/** The entry whose chat a global chat bot answers in: Flux for `null`, else the bot's own. */
export function dockSelectionForChatBot(botId: string | null): DockSelection {
	if (botId === null) return { type: "system", system: "flux" };
	if (botId === ASSISTANT_BOT_ID) return DEFAULT_DOCK_SELECTION;
	return { type: "bot", botId };
}

/** The Bots tab URL that opens a selection, with its panel on `panel` when one is given. */
export function dockRoute(
	selection: DockSelection,
	panel: BotPanelTab | null = null,
): string {
	const query = dockPanelQuery(panel, dockSelectionQuery(selection, ""));
	return query ? `${BOTS_ROUTE}?${query}` : BOTS_ROUTE;
}

/** The Bots tab URL that opens a selection along with the review that moves bots to the account. */
export function dockMoveRoute(selection: DockSelection): string {
	const params = new URLSearchParams(dockSelectionQuery(selection, ""));
	params.set("move", "local");
	return `${BOTS_ROUTE}?${params}`;
}

export function dockSelectionKey(selection: DockSelection): string {
	switch (selection.type) {
		case "system":
			return `system:${selection.system}`;
		case "bot":
			return `bot:${selection.botId}`;
		case "conversation":
			return `conversation:${selection.conversationId}`;
	}
}
