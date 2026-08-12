import type { LucideIcon } from "lucide-react";

/**
 * One conversation as the history list renders it. Every backing store — the global chat's Dexie
 * sessions, the per-app chat sessions, FlowPilot's conversations — normalizes into this shape at
 * its own call site, so the list itself never touches a database or knows which one it is showing.
 */
export interface IHistoryEntry {
	id: string;
	title: string;
	/** Epoch millis. Stores that keep ISO strings convert on the way in. */
	updatedAt: number;
	/** Epoch millis the user pinned this; absent means unpinned. */
	pinnedAt?: number;
	/** Optional second-line detail, e.g. "12 messages" or a board name. */
	subtitle?: string;
	/** Marks a conversation with a live run: shows a pulse and guards destructive actions. */
	streaming?: boolean;
	/** Optional leading glyph, e.g. FlowPilot's per-mode icon. */
	icon?: LucideIcon;
	/** Full-text body used for search only; never rendered. */
	searchBody?: string;
}

/** A date bucket ("Pinned", "Today", …) with the entries that fall in it. */
export interface IHistoryGroup {
	key: string;
	label: string;
	entries: IHistoryEntry[];
	pinned?: boolean;
}

export interface IChatHistoryListProps {
	/** `undefined` renders the loading skeleton; `[]` renders the empty state. */
	entries: readonly IHistoryEntry[] | undefined;
	activeId?: string;
	onSelect: (id: string) => void | Promise<void>;
	onNew?: () => void;
	/**
	 * Omitting a handler hides that affordance entirely, so a surface that cannot rename or pin
	 * simply does not pass one — no capability flags to keep in sync.
	 */
	onTogglePin?: (id: string, pinned: boolean) => void | Promise<void>;
	onRename?: (id: string, title: string) => void | Promise<void>;
	onDelete?: (id: string) => void | Promise<void>;
	/** Live search text is owned by the list; this only seeds the placeholder. */
	searchPlaceholder?: string;
	/**
	 * `comfortable` keeps row actions permanently visible and enlarges touch targets. Callers pass
	 * this from `useIsMobile()`; the list never queries the viewport itself.
	 */
	density?: "comfortable" | "compact";
	/** Rendered above the list, e.g. a conversation count. */
	header?: React.ReactNode;
	emptyTitle?: string;
	emptyDescription?: string;
	/**
	 * Fires when the user starts / stops searching. Callers use it to load message bodies into
	 * `searchBody` only while they are needed, instead of on every mount.
	 */
	onSearchActiveChange?: (active: boolean) => void;
	/**
	 * Fires while a row is being renamed. Hosts that dismiss on Escape (Popover, Sheet) must guard
	 * that dismissal, or Escape tears the whole surface down instead of cancelling the rename.
	 */
	onRenamingChange?: (renaming: boolean) => void;
	className?: string;
}
