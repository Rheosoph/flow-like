import type { BotAppearance } from "@flow-like/flow-like-ui/components/bots/types";
import type {
	BotApprovalDecision,
	BotTurnStatus,
} from "@flow-like/flow-like-ui/state/backend-state/bot-state";
import type {
	IInboxApproval,
	IInboxDesktopTarget,
} from "@flow-like/flow-like-ui/state/backend-state/inbox-state";

export const NOTCH_WINDOW_LABEL = "bot-notch";
export const NOTCH_ROUTE = "/notch";

/** Rust → every webview. */
export const NOTCH_EVENTS = {
	layout: "bot-notch://layout",
	settings: "bot-notch://settings",
	pointer: "bot-notch://pointer",
	frames: "bot-notch://frames",
	turn: "bot-host://turn",
} as const;

/** Between the notch and the main window. */
export const BRIDGE_EVENTS = {
	snapshot: "bot-notch://snapshot",
	command: "bot-notch://command",
	result: "bot-notch://result",
} as const;

/** Window-local logical pixels of the notch window. */
export interface NotchRect {
	x: number;
	y: number;
	width: number;
	height: number;
}

export interface NotchPoint {
	x: number;
	y: number;
}

/** `bot-notch://pointer`: the cursor near the notch's shapes; `near: false` once it left. */
export interface NotchPointer extends NotchPoint {
	near: boolean;
	/** The cursor touches the display's top edge while the den hides in a fullscreen Space. */
	edge: boolean;
}

export interface NotchLayout {
	width: number;
	height: number;
	scale: number;
	menuBarHeight: number;
	/** The hardware notch: no pixels exist between `left` and `right` above `height`. */
	notch: { left: number; right: number; height: number } | null;
	/** The display shows a fullscreen Space: the den hides until the cursor reaches the top edge. */
	fullscreen: boolean;
	platform: "macos" | "windows" | "linux";
}

export interface NotchSettings {
	enabled: boolean;
	hiddenBots: string[];
}

export type BrowserKind =
	| "chrome"
	| "chromium"
	| "edge"
	| "brave"
	| "arc"
	| "vivaldi"
	| "opera"
	| "safari"
	| "firefox";

export interface NotchWindow {
	/** The id `computer_windows` reports. */
	id: string;
	pid: number;
	app: string;
	title: string | null;
	bounds: NotchRect;
	browser: BrowserKind | null;
}

export interface NotchTab {
	url: string | null;
	title: string | null;
}

export interface PerchSpec {
	botId: string;
	windowId: string;
	/** Where the bot sits along the window's top edge, from its left. */
	anchorX: number;
	tab: NotchTab | null;
}

export interface PerchFrame {
	botId: string;
	windowId: string;
	/** The last bounds while the window is off screen; null once it is gone. */
	bounds: NotchRect | null;
	/** On screen, not minimized and its perch point not covered by another window. */
	visible: boolean;
	/** For a tab perch: the tab is the window's active one. Always true for a window. */
	tabActive: boolean;
	/** The window was closed. */
	gone: boolean;
}

/** A turn of the desktop's bot host, as `bot-host://turn` carries it. */
export interface HostTurn {
	id: string;
	bot_id: string;
	conversation_id: string;
	trigger_message_id: string;
	status: BotTurnStatus;
	started_at?: string | null;
	finished_at?: string | null;
	error_code?: string | null;
	reply_message_id?: string | null;
	status_text?: string | null;
}

/** A bot the notch may show, as the main window describes it. */
export interface NotchBot {
	id: string;
	/** The visibility key: a built-in bot's system key, any other bot's id. */
	key: string;
	name: string;
	appearance: BotAppearance;
	/** Holds `computer_use`: can see and operate windows. */
	computer: boolean;
	/** Holds `browser_use`: can open and drive pages. */
	browser: boolean;
	/** Acts without asking: the bot holds the auto-approval grant. */
	autoApprove: boolean;
}

export interface NotchSnapshot {
	/** The main window has loaded the bots; until then an empty list means nothing. */
	ready: boolean;
	bots: NotchBot[];
}

export interface NotchEntry {
	text: string | null;
	approval: IInboxApproval | null;
	/** The card as the entry carries it. */
	card: IInboxApproval | null;
	errorCode: string | null;
}

export type NotchCommand =
	| { kind: "snapshot" }
	| {
			kind: "send";
			botId: string;
			messageId: string;
			text: string;
			pointedAt: IInboxDesktopTarget | null;
	  }
	| { kind: "entry"; conversationId: string; messageId: string }
	| {
			kind: "answer";
			botId: string;
			turnId: string;
			approvalId: string;
			conversationId: string;
			approval: IInboxApproval;
			decision: BotApprovalDecision;
	  }
	/** Shows the main window at the bot's chat, or at the Bots tab without a bot. */
	| { kind: "open"; botId: string | null; conversationId: string | null }
	/** Switches the bot's auto approval (the `(*, *)` grant every host honours). */
	| { kind: "autoApprove"; botId: string; enabled: boolean }
	| { kind: "stop"; turn: HostTurn };

export interface NotchCommandEnvelope {
	id: string;
	command: NotchCommand;
}

export type NotchCommandResult =
	| { id: string; ok: true; data: unknown }
	| { id: string; ok: false; error: string };

export interface NotchSendResult {
	conversationId: string;
}
