export const GUIDE_EVENT = "flow:guide-event";

export type GuideEventName =
	| "bots:chat-opened"
	| "bots:message-sent"
	| "bots:reply-finished"
	| "bots:approval-shown"
	| "bots:approval-answered"
	| "bots:details-opened"
	| "store:app-opened"
	| "library:create-opened"
	| "library:app-created"
	| "flows:create-opened"
	| "flows:board-created"
	| "board:palette-opened"
	| "board:node-added"
	| "board:pins-connected"
	| "board:run-started"
	| "board:run-finished"
	| "board:logs-opened";

export interface GuideEventDetail {
	name: GuideEventName;
}

/** Tells a running guide that the user did something. Free when no tour runs. */
export function emitGuideEvent(name: GuideEventName): void {
	if (typeof window === "undefined") return;
	window.dispatchEvent(
		new CustomEvent<GuideEventDetail>(GUIDE_EVENT, { detail: { name } }),
	);
}

/** Reports the board edits a tour waits for, from commands that just committed. */
export function emitGuideEventsForCommands(
	commands: readonly { command_type: string }[],
): void {
	const kinds = new Set(commands.map((command) => command.command_type));
	if (kinds.has("AddNode")) emitGuideEvent("board:node-added");
	if (kinds.has("ConnectPin")) emitGuideEvent("board:pins-connected");
}

export function readGuideEvent(event: Event): GuideEventName | undefined {
	return (event as CustomEvent<Partial<GuideEventDetail>>).detail?.name;
}
