import {
	AppWindowIcon,
	BotIcon,
	ChartColumnIcon,
	ClipboardListIcon,
	DatabaseIcon,
	type LucideIcon,
	ShapesIcon,
	WaypointsIcon,
} from "lucide-react";
import type { CSSProperties } from "react";
import { IAppType } from "./schema/app/app";
import type { IBoardListing } from "./schema/flow/board-summary";
import type { IEvent } from "./schema/flow/event";

export interface AppTypeMeta {
	label: string;
	/** One line, written for the owner picking a type in config. */
	description: string;
	icon: LucideIcon;
	/**
	 * Outline of the app's icon tile. Type is encoded by silhouette rather than
	 * colour because colour is already spoken for — topic categories own a hue
	 * each, and visibility owns the coloured status badges.
	 *
	 * Radii are percentages so a mark reads the same at 24px and 64px.
	 */
	shape: CSSProperties;
}

// Radii are percentages so a mark reads the same at 24px and 64px. Clipped
// shapes carry no radius — the clip path is the outline.
const SHAPE_AGENT: CSSProperties = { borderRadius: "50%" };
const SHAPE_INTERFACE: CSSProperties = { borderRadius: "6% 6% 34% 34%" };
const SHAPE_DATA_FOCUS: CSSProperties = {
	clipPath: "polygon(50% 0%, 100% 25%, 100% 75%, 50% 100%, 0% 75%, 0% 25%)",
};
const SHAPE_PIPELINE: CSSProperties = {
	clipPath: "polygon(0 0, 78% 0, 100% 50%, 78% 100%, 0 100%, 18% 50%)",
};
const SHAPE_ANALYTICS: CSSProperties = { borderRadius: "32% 32% 32% 0" };
const SHAPE_FORM: CSSProperties = {
	clipPath: "polygon(0 0, 74% 0, 100% 26%, 100% 100%, 0 100%)",
};

/** Fallback for apps the owner has not classified yet. */
export const UNCLASSIFIED_APP_TYPE: AppTypeMeta = {
	label: "Unclassified",
	description: "No type set yet — pick one so it is recognisable at a glance.",
	icon: ShapesIcon,
	shape: { borderRadius: "22%" },
};

export const APP_TYPE_META: Record<IAppType, AppTypeMeta> = {
	[IAppType.Agent]: {
		label: "Agent",
		description: "Reacts to messages, mail or tickets and acts on its own.",
		icon: BotIcon,
		shape: SHAPE_AGENT,
	},
	[IAppType.CustomInterface]: {
		label: "Custom Interface",
		description: "A purpose-built screen people work inside.",
		icon: AppWindowIcon,
		shape: SHAPE_INTERFACE,
	},
	[IAppType.DataFocus]: {
		label: "Data Focus",
		description: "Owns a dataset — models it, curates it, serves it to others.",
		icon: DatabaseIcon,
		shape: SHAPE_DATA_FOCUS,
	},
	[IAppType.DataPipeline]: {
		label: "Data Pipeline",
		description: "Moves and transforms on a schedule. Usually no UI.",
		icon: WaypointsIcon,
		shape: SHAPE_PIPELINE,
	},
	[IAppType.Analytics]: {
		label: "Dashboards & Analytics",
		description: "Reads data back out — charts, queries and reporting.",
		icon: ChartColumnIcon,
		shape: SHAPE_ANALYTICS,
	},
	[IAppType.Form]: {
		label: "Form",
		description: "Collects structured input, then routes or files it.",
		icon: ClipboardListIcon,
		shape: SHAPE_FORM,
	},
};

export function appTypeMeta(type?: IAppType | null): AppTypeMeta {
	if (!type) return UNCLASSIFIED_APP_TYPE;
	return APP_TYPE_META[type] ?? UNCLASSIFIED_APP_TYPE;
}

export function appTypeLabel(type?: IAppType | null): string {
	return appTypeMeta(type).label;
}

/** Ordered for the config dropdown — most common first. */
export const APP_TYPE_ORDER: IAppType[] = [
	IAppType.Agent,
	IAppType.CustomInterface,
	IAppType.DataPipeline,
	IAppType.Analytics,
	IAppType.DataFocus,
	IAppType.Form,
];

const CHAT_EVENTS = new Set(["simple_chat", "discord", "telegram", "email"]);
const FORM_EVENTS = new Set(["generic_form"]);
const HEADLESS_EVENTS = new Set([
	"cron",
	"api",
	"rest",
	"mcp",
	"http",
	"daemon",
]);

/**
 * Best guess at an app's type from what it actually contains. Only ever used as
 * the *default* for the config dropdown — the owner's explicit choice always
 * wins, and a guess is never persisted on its own.
 *
 * Returns null when the evidence is too thin to call, so an empty project is
 * offered a free choice instead of being labelled at random.
 */
export function detectAppType(
	boards: readonly Pick<IBoardListing, "nodeCount">[] | undefined,
	events: IEvent[] | undefined,
	pageCount: number,
	tableCount = 0,
): IAppType | null {
	const activeEvents = (events ?? []).filter((event) => event.active);
	const eventTypes = new Set(activeEvents.map((event) => event.event_type));
	const hasLogic = (boards ?? []).some((board) => board.nodeCount > 0);

	if (!hasLogic && pageCount === 0 && tableCount === 0) return null;

	if ([...eventTypes].some((type) => FORM_EVENTS.has(type))) {
		return IAppType.Form;
	}
	if ([...eventTypes].some((type) => CHAT_EVENTS.has(type))) {
		return IAppType.Agent;
	}
	if (pageCount > 0) {
		return IAppType.CustomInterface;
	}
	if ([...eventTypes].some((type) => HEADLESS_EVENTS.has(type))) {
		return IAppType.DataPipeline;
	}
	if (tableCount > 0) {
		return IAppType.DataFocus;
	}
	return null;
}
