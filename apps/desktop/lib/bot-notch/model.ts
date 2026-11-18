import { botVisibilityKey } from "@flow-like/flow-like-ui/components/bots-tab/bot-visibility-model";
import type {
	BotAgentState,
	BotMood,
} from "@flow-like/flow-like-ui/components/bots/types";
import {
	type DenShape,
	PERCH_BLOB,
} from "@flow-like/flow-like-ui/lib/bot-goo/scene";
import { pointedAtLabel } from "@flow-like/flow-like-ui/lib/pointed-at";
import type { IBot } from "@flow-like/flow-like-ui/state/backend-state/bot-state";
import type { IInboxDesktopTarget } from "@flow-like/flow-like-ui/state/backend-state/inbox-state";
import type {
	HostTurn,
	NotchBot,
	NotchLayout,
	NotchPoint,
	NotchRect,
	NotchTab,
	NotchWindow,
} from "./protocol";

const DEN_SLOT = 32;
const DEN_PAD = 14;
const EAR_INNER = 8;
const DEN_MIN_HEIGHT = 30;
const DEN_HOVER_GROW = 6;
/** macOS keeps the window buttons in the first pixels of a title bar. */
const TITLE_BUTTONS = 84;
const PERCH_SPACING = 46;
const RECENT_MS = 8_000;

/** Bots that act on this computer's windows or browser: the notch's default crew. */
export function notchEligible(
	bot: Pick<IBot, "capabilities" | "location" | "local">,
): boolean {
	const here = bot.local === true || bot.location === "desktop";
	return (
		here &&
		bot.capabilities.some(
			(capability) =>
				capability === "computer_use" || capability === "browser_use",
		)
	);
}

export function notchBotOf(
	bot: IBot,
	name: string,
	autoApprove = false,
): NotchBot {
	return {
		id: bot.id,
		key: botVisibilityKey(bot),
		name,
		appearance: bot.appearance,
		computer: bot.capabilities.includes("computer_use"),
		browser: bot.capabilities.includes("browser_use"),
		autoApprove,
	};
}

/** The bots the profile shows that may sit in the notch, in the crew's order. */
export function notchCandidates(
	bots: readonly IBot[],
	profileHidden: ReadonlySet<string>,
): IBot[] {
	return bots.filter(
		(bot) => notchEligible(bot) && !profileHidden.has(botVisibilityKey(bot)),
	);
}

export function notchBotsOf(
	bots: readonly IBot[],
	profileHidden: ReadonlySet<string>,
	nameOf: (bot: IBot) => string,
	autoApproving: ReadonlySet<string> = new Set(),
): NotchBot[] {
	return notchCandidates(bots, profileHidden).map((bot) =>
		notchBotOf(bot, nameOf(bot), autoApproving.has(bot.id)),
	);
}

export function shownInNotch(
	bots: readonly NotchBot[],
	hiddenKeys: readonly string[],
): NotchBot[] {
	if (hiddenKeys.length === 0) return [...bots];
	const hidden = new Set(hiddenKeys);
	return bots.filter((bot) => !hidden.has(bot.key));
}

/** A plain window needs a bot that sees the screen; a browser tab also suits one that drives a browser. */
export function acceptsTarget(
	bot: Pick<NotchBot, "computer" | "browser">,
	window: Pick<NotchWindow, "browser">,
): boolean {
	return window.browser ? bot.computer || bot.browser : bot.computer;
}

export function pointedAtOf(
	window: NotchWindow,
	tab: NotchTab | null,
	hostId: string,
): IInboxDesktopTarget {
	const title = (tab ? (tab.title ?? window.title) : window.title)?.trim();
	const url = tab?.url?.trim();
	return {
		kind: window.browser ? "tab" : "window",
		host_id: hostId,
		window_id: window.id,
		app: window.app.trim() || "App",
		...(title ? { title: title.slice(0, 500) } : {}),
		...(window.browser && url ? { url: url.slice(0, 4096) } : {}),
	};
}

function clip(text: string, max: number): string {
	return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

/** What the drop label and the prompt's caption call a window or tab; the chat names it alike. */
export function targetLabel(window: NotchWindow, tab: NotchTab | null): string {
	return pointedAtLabel(pointedAtOf(window, tab, ""), 48);
}

function denHeight(layout: NotchLayout, hover: boolean): number {
	const band = layout.notch
		? layout.notch.height
		: Math.max(layout.menuBarHeight + 6, DEN_MIN_HEIGHT);
	return band + (hover ? DEN_HOVER_GROW : 0);
}

/**
 * The den's outline and where each bot sits in it. Tucked, it lies flat in the top edge and its
 * bots are above the screen with it.
 */
export function denShape(
	layout: NotchLayout,
	botIds: readonly string[],
	hover: boolean,
	tucked = false,
): DenShape {
	const resting = denHeight(layout, false);
	const height = tucked ? 0 : denHeight(layout, hover);
	const grow = hover && !tucked ? 4 : 0;
	const cy = resting / 2 + 0.5 - (tucked ? resting : 0);
	const notch = layout.notch;
	if (notch) {
		const right = Math.ceil(botIds.length / 2);
		const ear = Math.max(right, 1) * DEN_SLOT + DEN_PAD + EAR_INNER + grow;
		const slots = botIds.map((botId, index) => {
			const offset =
				EAR_INNER + DEN_SLOT / 2 + Math.floor(index / 2) * DEN_SLOT;
			const x = index % 2 === 0 ? notch.right + offset : notch.left - offset;
			return { botId, x, y: cy };
		});
		return {
			edge: "top",
			rect: {
				x: notch.left - ear,
				y: 0,
				width: notch.right - notch.left + ear * 2,
				height,
			},
			radius: Math.min(height / 2, 16),
			slots,
			wrapsNotch: true,
		};
	}
	const width = Math.max(botIds.length * DEN_SLOT + DEN_PAD * 2 + grow * 2, 64);
	const x = (layout.width - width) / 2;
	const first = x + (width - botIds.length * DEN_SLOT) / 2 + DEN_SLOT / 2;
	return {
		edge: "top",
		rect: { x, y: 0, width, height },
		radius: Math.min(height / 2, 15),
		slots: botIds.map((botId, index) => ({
			botId,
			x: first + index * DEN_SLOT,
			y: cy,
		})),
		wrapsNotch: false,
	};
}

/**
 * Whether something calls a hidden den out of the top edge: one of `signals` (the pointer at the
 * edge, a gesture), or a bot at home that is `attended` with a card, a prompt, a menu or the
 * pointer. `null` attends the den itself; a bot on a window leaves the den where it is.
 */
export function denCalledOut(
	signals: readonly boolean[],
	attended: readonly (string | null | undefined)[],
	perched: { has(botId: string): boolean },
): boolean {
	const atHome = (botId: string | null | undefined) =>
		botId === null || (botId !== undefined && !perched.has(botId));
	return signals.some(Boolean) || attended.some(atHome);
}

export function contains(rect: NotchRect, point: NotchPoint, margin = 0) {
	return (
		point.x >= rect.x - margin &&
		point.x <= rect.x + rect.width + margin &&
		point.y >= rect.y - margin &&
		point.y <= rect.y + rect.height + margin
	);
}

/**
 * Where along a window's top edge a dropped bot sits: under the drop, clear of the window buttons
 * and of the bots already there.
 */
export function perchAnchor(
	bounds: NotchRect,
	dropX: number,
	taken: readonly number[],
): number {
	const low = Math.min(TITLE_BUTTONS, bounds.width / 2);
	const high = Math.max(low, bounds.width - 36);
	let anchor = Math.min(Math.max(dropX - bounds.x, low), high);
	for (let attempt = 0; attempt < taken.length + 1; attempt++) {
		const clash = taken.find(
			(other) => Math.abs(other - anchor) < PERCH_SPACING,
		);
		if (clash === undefined) return anchor;
		const right = clash + PERCH_SPACING;
		anchor = right <= high ? right : Math.max(low, clash - PERCH_SPACING);
	}
	return anchor;
}

/** The centre of a perched bot: on the window's top edge, or just inside it under the menu bar. */
export function perchCenter(
	bounds: NotchRect,
	anchorX: number,
	topLimit: number,
): NotchPoint {
	const x = bounds.x + anchorX;
	const above = bounds.y - PERCH_BLOB * 0.62;
	return above - PERCH_BLOB < topLimit
		? { x, y: Math.max(bounds.y, topLimit) + PERCH_BLOB + 4 }
		: { x, y: above };
}

export interface BotActivity {
	agentState: BotAgentState;
	mood?: BotMood;
	turn: HostTurn | null;
}

function finishedAt(turn: HostTurn): number {
	const at = turn.finished_at ? Date.parse(turn.finished_at) : Number.NaN;
	return Number.isFinite(at) ? at : 0;
}

/** What a bot's face shows: the turn it works on now, or one that just ended. */
export function botActivity(
	turns: Iterable<HostTurn>,
	botId: string,
	now: number,
): BotActivity {
	let recent: HostTurn | null = null;
	for (const turn of turns) {
		if (turn.bot_id !== botId) continue;
		if (turn.status === "waiting") return { agentState: "needs-you", turn };
		if (turn.status === "running") return { agentState: "working", turn };
		if (turn.status === "queued")
			return { agentState: "idle", mood: "thinking", turn };
		if (
			(turn.status === "done" || turn.status === "failed") &&
			now - finishedAt(turn) < RECENT_MS &&
			(!recent || finishedAt(turn) > finishedAt(recent))
		)
			recent = turn;
	}
	if (!recent) return { agentState: "idle", turn: null };
	return {
		agentState: recent.status === "done" ? "done" : "error",
		turn: recent,
	};
}

/** A few lines of a markdown reply as plain text. */
export function previewText(markdown: string, max = 220): string {
	const plain = markdown
		.replace(/```[\s\S]*?```/g, " ")
		.replace(/!\[[^\]]*]\([^)]*\)/g, " ")
		.replace(/\[([^\]]+)]\([^)]*\)/g, "$1")
		.replace(/[*`>#~|]+/g, "")
		.replace(/(?<![\p{L}\p{N}])_+|_+(?![\p{L}\p{N}])/gu, "")
		.replace(/\s+/g, " ")
		.trim();
	return clip(plain, max);
}
