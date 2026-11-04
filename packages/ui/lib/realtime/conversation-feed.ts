import type { IInboxEntry } from "../../state/backend-state/inbox-state";
import { compareInboxCursors } from "../inbox-cursor";

export interface ConversationWatch {
	readonly conversationId: string;
	readonly as?: string;
	readonly after?: string;
}

/** New entries of a watched conversation, oldest first; the feed tracks the cursor itself. */
export type ConversationFeedListener = (
	entries: readonly IInboxEntry[],
) => void;

export interface ConversationFeed {
	watch(
		target: ConversationWatch,
		listener: ConversationFeedListener,
	): () => void;
}

interface Watcher {
	readonly target: ConversationWatch;
	readonly listener: ConversationFeedListener;
	cursor?: string;
	detach?: () => void;
}

const watchers = new Set<Watcher>();
let current: ConversationFeed | null = null;

function attach(watcher: Watcher): void {
	watcher.detach?.();
	watcher.detach = current?.watch(
		{ ...watcher.target, after: watcher.cursor },
		(entries) => {
			const fresh = entries.filter(
				(entry) =>
					watcher.cursor === undefined ||
					compareInboxCursors(entry.cursor, watcher.cursor) > 0,
			);
			const newest = fresh.at(-1);
			if (!newest) return;
			watcher.cursor = newest.cursor;
			watcher.listener(fresh);
		},
	);
}

export function registerConversationFeed(feed: ConversationFeed | null): void {
	if (feed === current) return;
	current = feed;
	for (const watcher of watchers) {
		if (feed) attach(watcher);
		else {
			watcher.detach?.();
			watcher.detach = undefined;
		}
	}
}

/** No-op unsubscribe when no feed is registered; watchers registered earlier attach when a feed registers, and move to the next feed when it changes. */
export function watchConversation(
	target: ConversationWatch,
	listener: ConversationFeedListener,
): () => void {
	const watcher: Watcher = { target, listener, cursor: target.after };
	watchers.add(watcher);
	attach(watcher);
	return () => {
		watchers.delete(watcher);
		watcher.detach?.();
		watcher.detach = undefined;
	};
}

export function hasConversationFeed(): boolean {
	return current !== null;
}
