import type { IInboxEntry } from "../../state/backend-state/inbox-state";
import { compareInboxCursors } from "../inbox-cursor";
import type {
	ConversationFeed,
	ConversationFeedListener,
	ConversationWatch,
} from "./conversation-feed";

interface HubWatcher {
	readonly conversationId: string;
	readonly as?: string;
	readonly listener: ConversationFeedListener;
	cursor?: string;
}

/** Turns delivered entries into the form listeners read, e.g. opens sealed ones. */
export type FeedEntryOpener = (
	entries: IInboxEntry[],
	as?: string,
) => Promise<IInboxEntry[]>;

/**
 * Bookkeeping shared by the bridges that back a feed: watchers with their own cursors, the merged
 * set of targets, and routing of delivered entries. `changed` fires once per microtask batch of
 * watch/unwatch calls.
 */
export class ConversationFeedHub implements ConversationFeed {
	private readonly watchers = new Set<HubWatcher>();
	private scheduled = false;
	private opener?: FeedEntryOpener;
	private delivering: Promise<void> = Promise.resolve();

	constructor(private readonly changed: () => void) {}

	/**
	 * Listeners get entries through `opener` from now on (end-to-end encrypted chats arrive
	 * sealed). Deliveries then wait for it in arrival order, so a slow open never lets a later
	 * batch move a cursor past an earlier one.
	 */
	setOpener(opener: FeedEntryOpener | undefined): void {
		this.opener = opener;
	}

	watch(
		target: ConversationWatch,
		listener: ConversationFeedListener,
	): () => void {
		const watcher: HubWatcher = {
			conversationId: target.conversationId,
			as: target.as,
			listener,
			cursor: target.after,
		};
		this.watchers.add(watcher);
		this.schedule();
		return () => {
			if (this.watchers.delete(watcher)) this.schedule();
		};
	}

	/** One target per conversation and reader at its oldest cursor; none when any watcher holds no entries. */
	targets(): ConversationWatch[] {
		const merged = new Map<
			string,
			{ target: ConversationWatch; open: boolean }
		>();
		for (const { conversationId, as, cursor } of this.watchers) {
			const key = JSON.stringify([conversationId, as ?? null]);
			const found = merged.get(key);
			if (!found) {
				merged.set(key, {
					target: { conversationId, as, after: cursor },
					open: cursor === undefined,
				});
				continue;
			}
			if (found.open) continue;
			if (
				cursor === undefined ||
				(found.target.after !== undefined &&
					compareInboxCursors(cursor, found.target.after) < 0)
			)
				found.target = { conversationId, as, after: cursor };
			found.open = cursor === undefined;
		}
		return [...merged.values()].map(({ target }) => target);
	}

	isWatched(conversationId: string, as?: string): boolean {
		for (const watcher of this.watchers)
			if (watcher.conversationId === conversationId && watcher.as === as)
				return true;
		return false;
	}

	/** Entries oldest first; without `as` every reader of the conversation receives them. */
	deliver(
		conversationId: string,
		entries: readonly IInboxEntry[],
		reader?: { as?: string },
	): void {
		const opener = this.opener;
		if (!opener) {
			this.route(conversationId, entries, reader);
			return;
		}
		this.delivering = this.delivering.then(async () => {
			let opened: readonly IInboxEntry[] = entries;
			try {
				opened = await opener([...entries], reader?.as);
			} catch (error) {
				console.warn("Opening delivered conversation entries failed", error);
			}
			this.route(conversationId, opened, reader);
		});
	}

	private route(
		conversationId: string,
		entries: readonly IInboxEntry[],
		reader?: { as?: string },
	): void {
		for (const watcher of [...this.watchers]) {
			if (
				watcher.conversationId !== conversationId ||
				(reader && watcher.as !== reader.as)
			)
				continue;
			const fresh = entries.filter(
				(entry) =>
					watcher.cursor === undefined ||
					compareInboxCursors(entry.cursor, watcher.cursor) > 0,
			);
			const newest = fresh.at(-1);
			if (!newest) continue;
			watcher.cursor = newest.cursor;
			try {
				watcher.listener(fresh);
			} catch (error) {
				console.warn("Conversation feed listener failed", error);
			}
		}
	}

	private schedule(): void {
		if (this.scheduled) return;
		this.scheduled = true;
		queueMicrotask(() => {
			this.scheduled = false;
			this.changed();
		});
	}
}
