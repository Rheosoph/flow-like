import type { IChatMainWidget, IMessage } from "./chat-db";

let latestRevision = 0;

/** Strictly increasing even when several events arrive within one millisecond. */
export function nextMainWidgetRevision(previous?: IChatMainWidget): number {
	latestRevision = Math.max(
		Date.now(),
		latestRevision + 1,
		(previous?.updated_at ?? 0) + 1,
	);
	return latestRevision;
}

/** Continue a command from its newest action feedback before folding another run event. */
export function refreshMainChatWidget(
	message: IMessage,
	latest?: IChatMainWidget,
): void {
	const current = message.main_widget;
	if (
		!current?.command_id ||
		!current.widget ||
		!latest?.widget ||
		current.command_id !== latest.command_id ||
		current.widget.instance_id !== latest.widget.instance_id ||
		(latest.updated_at ?? 0) <= (current.updated_at ?? 0)
	) {
		return;
	}
	message.main_widget = latest;
}

/** Resolve the pane from chronological history, including an unfinished turn. */
export function resolveMainChatWidget(
	messages: readonly IMessage[],
	sessionId: string,
	liveMessage?: IMessage,
): IChatMainWidget | undefined {
	let latest: IChatMainWidget | undefined;
	let latestCommandTime = Number.NEGATIVE_INFINITY;
	let latestTime = Number.NEGATIVE_INFINITY;
	const consider = (message: IMessage) => {
		if (message.sessionId !== sessionId || !message.main_widget) return;
		const time = message.main_widget.updated_at ?? message.timestamp;
		const commandTime = message.main_widget.command_at ?? time;
		if (
			commandTime < latestCommandTime ||
			(commandTime === latestCommandTime && time < latestTime)
		) {
			return;
		}
		latest = message.main_widget;
		latestCommandTime = commandTime;
		latestTime = time;
	};
	for (const message of messages) {
		consider(message);
	}
	if (liveMessage) consider(liveMessage);
	return latest;
}
