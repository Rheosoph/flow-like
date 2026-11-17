import { botVisibilityKey } from "../../components/bots-tab/bot-visibility-model";
import type { BotAgentState, BotAppearance } from "../../components/bots/types";
import {
	FLOWPILOT_CAPABILITY,
	type IBot,
} from "../../state/backend-state/bot-state";
import { conversationBotId } from "../../state/global-chat/global-chat-bots";
import type {
	GlobalChatRun,
	GlobalToolPrompt,
} from "../../state/global-chat/global-chat-store";
import {
	type DenShape,
	type GooPoint,
	type GooRect,
	PERCH_BLOB,
} from "../bot-goo/scene";

/** Flux's key in the den: it has no row id while it has no account row. */
export const FLUX_DEN_KEY = "flux";

const DEN_SLOT = 32;
const DEN_PAD = 14;
const DEN_DEPTH = 30;
const DEN_HOVER_GROW = 6;
/** Where the den rests: this far above the bottom of its edge, like the old bubble. */
const DEN_LIFT = 132;
const AVOID_GAP = 16;
const EDGE_MARGIN = 12;
/** A perched bot keeps this share of its target's width away from either end. */
const PERCH_END = 0.12;
/** Taller targets are areas, not things: a bot dropped on one stays where it was dropped. */
const AREA_HEIGHT = 200;

export interface DenBot {
	/** Its key in the den: `flux` or the bot's id. */
	key: string;
	/** The global chat bot it answers as: null for Flux. */
	chatBotId: string | null;
	name: string;
	appearance: Partial<BotAppearance>;
}

/**
 * The den's crew: the builders. Flux always, then every duplicate of Flux the profile shows; a
 * bot without the `flowpilot` toolset can't build, so it lives in the notch or the Bots tab.
 */
export function denCrew(
	bots: readonly IBot[],
	hidden: ReadonlySet<string>,
	flux: Pick<DenBot, "name" | "appearance">,
	nameOf: (bot: IBot) => string,
): DenBot[] {
	const builders = bots
		.filter(
			(bot) =>
				bot.system === null &&
				bot.capabilities.includes(FLOWPILOT_CAPABILITY) &&
				!hidden.has(botVisibilityKey(bot)),
		)
		.map((bot) => ({
			key: bot.id,
			chatBotId: bot.id,
			name: nameOf(bot),
			appearance: bot.appearance,
		}));
	return [{ key: FLUX_DEN_KEY, chatBotId: null, ...flux }, ...builders];
}

/** The den key of the bot a global chat conversation belongs to. */
export function denKeyOfConversation(conversationId: string): string {
	return conversationBotId(conversationId) ?? FLUX_DEN_KEY;
}

export interface DenActivity {
	agentState: BotAgentState;
}

/** What each den bot's face shows, from the global chat's live runs and its open prompt. */
export function denActivities(
	keys: readonly string[],
	runs: Readonly<Record<string, Pick<GlobalChatRun, "conversationId">>>,
	toolPrompt: Pick<GlobalToolPrompt, "conversationId"> | null,
): Map<string, DenActivity> {
	const working = new Set(
		Object.values(runs).map((run) => denKeyOfConversation(run.conversationId)),
	);
	const asking = toolPrompt?.conversationId
		? denKeyOfConversation(toolPrompt.conversationId)
		: null;
	return new Map(
		keys.map((key) => [
			key,
			{
				agentState:
					asking === key ? "needs-you" : working.has(key) ? "working" : "idle",
			},
		]),
	);
}

/** Where the den may sit: the right edge of its surface, between that surface's top and bottom. */
export interface DenBounds {
	right: number;
	top: number;
	bottom: number;
}

export function denLength(count: number, hover: boolean): number {
	return Math.max(
		count * DEN_SLOT + DEN_PAD * 2 + (hover ? 8 : 0),
		DEN_SLOT + DEN_PAD * 2,
	);
}

/**
 * The den's centre along its edge: low on the edge like the old bubble, and above anything that
 * covers that edge (the chat overlay). Null when nothing leaves it room: it tucks away.
 */
export function denCenter(
	bounds: DenBounds,
	length: number,
	avoid: readonly GooRect[],
): number | null {
	const half = length / 2;
	const lowest = bounds.bottom - EDGE_MARGIN - half;
	const highest = bounds.top + EDGE_MARGIN + half;
	let center = Math.max(highest, bounds.bottom - DEN_LIFT - half);
	for (const rect of [...avoid].sort((a, b) => b.y - a.y)) {
		const covers =
			rect.x <= bounds.right - 4 && rect.x + rect.width >= bounds.right - 40;
		if (!covers) continue;
		const overlaps =
			center + half + AVOID_GAP > rect.y &&
			center - half - AVOID_GAP < rect.y + rect.height;
		if (overlaps) center = rect.y - AVOID_GAP - half;
	}
	if (center < highest || center > lowest + DEN_LIFT) return null;
	return Math.min(center, lowest);
}

/** The den on the right edge at `right`, centred at `center`, its bots stacked along the edge. */
export function sideDenShape(
	right: number,
	center: number,
	keys: readonly string[],
	hover: boolean,
): DenShape {
	const depth = DEN_DEPTH + (hover ? DEN_HOVER_GROW : 0);
	const length = denLength(keys.length, hover);
	const first = center - (keys.length * DEN_SLOT) / 2 + DEN_SLOT / 2;
	const slotX = right - DEN_DEPTH / 2 - 0.5;
	return {
		edge: "right",
		rect: {
			x: right - depth,
			y: center - length / 2,
			width: depth,
			height: length,
		},
		radius: Math.min(depth / 2, 15),
		slots: keys.map((botId, index) => ({
			botId,
			x: slotX,
			y: first + index * DEN_SLOT,
		})),
		wrapsNotch: false,
	};
}

/** A board, a page or a long list is an area to sit in; a node or a row has an edge to sit on. */
export function isArea(rect: GooRect): boolean {
	return rect.height > AREA_HEIGHT;
}

function share(from: number, length: number, at: number, end: number): number {
	if (length <= 0) return 0.5;
	return Math.min(1 - end, Math.max(end, (at - from) / length));
}

/** Where on its target a dropped bot sits, as shares of the target's width and height. */
export function perchShare(rect: GooRect, drop: GooPoint): GooPoint {
	return {
		x: share(rect.x, rect.width, drop.x, PERCH_END),
		y: share(rect.y, rect.height, drop.y, 0.04),
	};
}

/**
 * A perched bot stands on its target's top edge, or sits just inside when that edge is too high;
 * in an area it stays where it was dropped.
 */
export function perchOn(
	rect: GooRect,
	at: GooPoint,
	topLimit: number,
): GooPoint & { edgeY: number | null } {
	const x = rect.x + rect.width * at.x;
	if (isArea(rect))
		return {
			x,
			y: Math.max(rect.y + rect.height * at.y, topLimit + PERCH_BLOB),
			edgeY: null,
		};
	const above = rect.y - PERCH_BLOB * 0.62;
	return above - PERCH_BLOB < topLimit
		? { x, y: Math.max(rect.y, topLimit) + PERCH_BLOB + 4, edgeY: null }
		: { x, y: above, edgeY: rect.y };
}

/** Whether a target is still on screen inside the den's surface. */
export function inView(rect: GooRect, bounds: DenBounds, left = 0): boolean {
	return (
		rect.width > 0 &&
		rect.x + rect.width > left &&
		rect.x < bounds.right &&
		rect.y + rect.height > bounds.top &&
		rect.y < bounds.bottom
	);
}
