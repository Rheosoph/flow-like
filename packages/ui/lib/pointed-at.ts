import type { IInboxDesktopTarget } from "../state/backend-state/inbox-state";

/** What inside Flow-Like a bot can be dropped on from the den. */
export const APP_POINTED_AT_KINDS = [
	"app",
	"board",
	"layer",
	"node",
	"page",
	"widget",
	"component",
	"event",
	"table",
	"storage",
	"comment",
	"pin",
	"variable",
	"run",
] as const;
export type AppPointedAtKind = (typeof APP_POINTED_AT_KINDS)[number];

/** A thing inside Flow-Like a message is about: the person dropped the bot on it. */
export interface IAppPointedAt {
	kind: AppPointedAtKind;
	app_id: string;
	/** The board, layer, page, widget, event, comment, pin, variable or run; a table's name; a
	 * storage path. */
	id?: string;
	/** Nodes or components dropped on together, as one selection; a pin's node. */
	ids?: string[];
	/** The board a layer or the nodes are on. */
	board_id?: string;
	/** The page the components are on. */
	page_id?: string;
	/** The widget the components are in. */
	widget_id?: string;
	/** What the person saw it called. */
	label: string;
}

/** A window or tab from the notch, or a thing inside Flow-Like from the den. */
export type IPointedAt = IInboxDesktopTarget | IAppPointedAt;

export function isAppPointedAt(target: IPointedAt): target is IAppPointedAt {
	return target.kind !== "window" && target.kind !== "tab";
}

function hostOf(url: string | undefined): string | null {
	if (!url) return null;
	try {
		return new URL(url).host || null;
	} catch {
		return null;
	}
}

function clip(text: string, max: number): string {
	return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

/**
 * How what a message points at is named: "Code — main.rs", "Pull requests · github.com", or the
 * name a node, board or page had on screen.
 */
export function pointedAtLabel(target: IPointedAt, max = 64): string {
	if (isAppPointedAt(target)) return clip(target.label.trim(), max);
	const title = target.title?.trim();
	if (target.kind === "tab") {
		const head = title ? clip(title, max) : target.app;
		const host = hostOf(target.url);
		return host ? `${head} · ${host}` : head;
	}
	return title ? `${target.app} — ${clip(title, max)}` : target.app;
}
