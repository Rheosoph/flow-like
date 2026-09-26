/**
 * What an editor tab is showing.
 *
 * The strip lists what is *open*; the explorer lists what *exists*. Before this the two
 * were the same thing — a tab was a bare module-layer id and the active tab was derived
 * from canvas navigation, so nothing but a `.flow` file could ever have one.
 *
 * A document is an identity, not a position. Where a board tab is parked inside its file
 * lives on `IEditorTab.layerPath`, because walking into a function does not open a
 * different document — it moves within one, the way scrolling does in a text editor.
 */

export type IEditorScope = "app" | "user";

export type IEditorDocument =
	| { kind: "board"; fileId: string }
	| { kind: "page"; pageId: string }
	| { kind: "widget"; widgetId: string }
	| { kind: "storage"; scope: IEditorScope; location: string }
	| { kind: "table"; scope: IEditorScope; table: string }
	/**
	 * The app-wide stylesheet. App-scoped rather than board-scoped, so it has no
	 * id and its document key is constant — one app has exactly one.
	 */
	| { kind: "styles" };

export type IEditorDocumentKind = IEditorDocument["kind"];

export const EDITOR_TAB_COLORS = [
	"red",
	"orange",
	"yellow",
	"green",
	"teal",
	"blue",
	"purple",
	"pink",
] as const;

export type IEditorTabColor = (typeof EDITOR_TAB_COLORS)[number];

export interface IEditorTab {
	/** Stable for the tab's life. Deduped by `documentKey`, suffixed for extra instances. */
	readonly key: string;
	readonly doc: IEditorDocument;
	/**
	 * Board tabs only: the layer path the tab is parked on, `undefined` for the file root.
	 * Restored when the tab is focused, so two tabs on one file can sit at different depths.
	 */
	readonly layerPath?: string;
	/**
	 * Pinned tabs are always a prefix of the strip — every mutator keeps them there — so
	 * strip order, render order and "the neighbour a close falls back to" stay one order.
	 */
	readonly pinned?: boolean;
	readonly color?: IEditorTabColor;
}

/** Identity of the *document*, ignoring where a tab is parked inside it. */
export function documentKey(doc: IEditorDocument): string {
	switch (doc.kind) {
		case "board":
			return `board:${doc.fileId}`;
		case "page":
			return `page:${doc.pageId}`;
		case "widget":
			return `widget:${doc.widgetId}`;
		case "storage":
			return `storage:${doc.scope}:${doc.location}`;
		case "table":
			return `table:${doc.scope}:${doc.table}`;
		case "styles":
			return "styles:app";
	}
}

export function sameDocument(a: IEditorDocument, b: IEditorDocument): boolean {
	return documentKey(a) === documentKey(b);
}

/**
 * A free key for a new tab on `doc`. The first is the document key itself, so the common
 * case round-trips through persistence unchanged and reads plainly in a test.
 */
export function nextTabKey(
	tabs: readonly IEditorTab[],
	doc: IEditorDocument,
): string {
	const base = documentKey(doc);
	const taken = new Set(tabs.map((tab) => tab.key));
	if (!taken.has(base)) return base;
	for (let instance = 2; ; instance += 1) {
		const candidate = `${base}#${instance}`;
		if (!taken.has(candidate)) return candidate;
	}
}

export function findTab(
	tabs: readonly IEditorTab[],
	doc: IEditorDocument,
): IEditorTab | undefined {
	return tabs.find((tab) => sameDocument(tab.doc, doc));
}

export function tabByKey(
	tabs: readonly IEditorTab[],
	key: string | null | undefined,
): IEditorTab | undefined {
	return key ? tabs.find((tab) => tab.key === key) : undefined;
}

export interface IOpenResult {
	readonly tabs: IEditorTab[];
	/** The tab to focus — an existing one unless a new instance was asked for. */
	readonly key: string;
}

/**
 * Open `doc`, reusing its tab when it already has one. `newTab` forces a second instance,
 * which is the only way to get one file open twice.
 */
export function withDocumentOpened(
	tabs: readonly IEditorTab[],
	doc: IEditorDocument,
	options?: { newTab?: boolean; layerPath?: string; after?: string },
): IOpenResult {
	if (!options?.newTab) {
		const existing = findTab(tabs, doc);
		if (existing) {
			const tabs_ =
				options?.layerPath === undefined
					? [...tabs]
					: withTabLayerPath(tabs, existing.key, options.layerPath);
			return { tabs: tabs_, key: existing.key };
		}
	}

	const tab: IEditorTab = {
		key: nextTabKey(tabs, doc),
		doc,
		...(options?.layerPath === undefined
			? {}
			: { layerPath: options.layerPath }),
	};
	const at = tabs.findIndex((other) => other.key === options?.after);
	const next = [...tabs];
	next.splice(
		Math.max(at === -1 ? tabs.length : at + 1, pinnedCount(tabs)),
		0,
		tab,
	);
	return { tabs: next, key: tab.key };
}

function pinnedCount(tabs: readonly IEditorTab[]): number {
	return tabs.filter((tab) => tab.pinned).length;
}

/** Stable partition, pinned first. Only needed where tabs arrive from outside the mutators. */
export function withPinnedFirst(tabs: readonly IEditorTab[]): IEditorTab[] {
	return [
		...tabs.filter((tab) => tab.pinned),
		...tabs.filter((tab) => !tab.pinned),
	];
}

/**
 * Pinning appends to the pinned group; unpinning puts the tab first among the rest. Both
 * land on the boundary, so the tab moves as little as the prefix rule allows.
 */
export function withTabPinned(
	tabs: readonly IEditorTab[],
	key: string,
	pinned: boolean,
): IEditorTab[] {
	const tab = tabByKey(tabs, key);
	if (!tab || Boolean(tab.pinned) === pinned) return [...tabs];
	const { pinned: _previous, ...rest } = tab;
	const moved: IEditorTab = pinned ? { ...rest, pinned: true } : rest;
	const others = tabs.filter((other) => other.key !== key);
	const boundary = pinnedCount(others);
	return [...others.slice(0, boundary), moved, ...others.slice(boundary)];
}

/** `undefined` clears the colour. */
export function withTabColor(
	tabs: readonly IEditorTab[],
	key: string,
	color: IEditorTabColor | undefined,
): IEditorTab[] {
	return tabs.map((tab) => {
		if (tab.key !== key || tab.color === color) return tab;
		const { color: _previous, ...rest } = tab;
		return color === undefined ? rest : { ...rest, color };
	});
}

export function withTabClosed(
	tabs: readonly IEditorTab[],
	key: string,
): IEditorTab[] {
	return tabs.filter((tab) => tab.key !== key);
}

/**
 * What to focus after closing `key`, given it was the tab on screen.
 *
 * The tab that slid into its place, else the one before it — the same rule an editor uses,
 * so closing a run of tabs walks left rather than throwing you back to the root each time.
 * Board tabs are preferred over the neighbour when the neighbour is gone, because the canvas
 * is what the shell falls back to.
 */
export function tabAfterClose(
	tabs: readonly IEditorTab[],
	key: string,
): string | null {
	const index = tabs.findIndex((tab) => tab.key === key);
	if (index === -1) return null;
	const remaining = withTabClosed(tabs, key);
	const next = remaining[index] ?? remaining[index - 1];
	if (next) return next.key;
	return remaining.find((tab) => tab.doc.kind === "board")?.key ?? null;
}

/**
 * The canvas is always showing something, so the last board tab cannot be closed. Every
 * other tab can — including a second instance of the same file.
 */
export function isTabClosable(
	tabs: readonly IEditorTab[],
	key: string,
): boolean {
	const tab = tabByKey(tabs, key);
	if (!tab) return false;
	if (tab.doc.kind !== "board") return true;
	return tabs.filter((other) => other.doc.kind === "board").length > 1;
}

/**
 * Tabs whose document the app no longer has cannot survive a delete elsewhere — a module
 * dropped from the board, a page deleted from the explorer, a file removed from storage.
 *
 * The predicate is per-document rather than per-id: the old model could prune against
 * `board.layers` alone because every tab *was* a layer.
 */
export function withMissingTabsDropped(
	tabs: readonly IEditorTab[],
	exists: (doc: IEditorDocument) => boolean,
): IEditorTab[] {
	return tabs.filter((tab) => exists(tab.doc));
}

/** Park a board tab on a layer path. `undefined` is the file root. */
export function withTabLayerPath(
	tabs: readonly IEditorTab[],
	key: string,
	layerPath: string | undefined,
): IEditorTab[] {
	return tabs.map((tab) => {
		if (tab.key !== key) return tab;
		if (tab.layerPath === layerPath) return tab;
		const { layerPath: _dropped, ...rest } = tab;
		return layerPath === undefined ? rest : { ...rest, layerPath };
	});
}

/** Keep a board tab's file and canvas position in the same update. */
export function withBoardTabPosition(
	tabs: IEditorTab[],
	key: string,
	fileId: string,
	layerPath: string | undefined,
): IEditorTab[] {
	const index = tabs.findIndex((tab) => tab.key === key);
	const tab = tabs[index];
	if (!tab || tab.doc.kind !== "board") return tabs;
	if (tab.doc.fileId === fileId && tab.layerPath === layerPath) return tabs;

	const { layerPath: _previousPath, ...rest } = tab;
	const doc = tab.doc.fileId === fileId ? tab.doc : { ...tab.doc, fileId };
	const next = [...tabs];
	next[index] = {
		...rest,
		doc,
		...(layerPath === undefined ? {} : { layerPath }),
	};
	return next;
}

/**
 * Board tabs on `fileId`, in strip order. Layer paths are per-tab, so navigating on the
 * canvas has to know which tab it is moving.
 */
export function boardTabsFor(
	tabs: readonly IEditorTab[],
	fileId: string,
): IEditorTab[] {
	return tabs.filter(
		(tab) => tab.doc.kind === "board" && tab.doc.fileId === fileId,
	);
}

const PERSIST_VERSION = 1;

interface IPersistedTabs {
	readonly v: number;
	readonly tabs: readonly IEditorTab[];
	readonly active?: string;
}

export function serializeTabs(
	tabs: readonly IEditorTab[],
	activeKey: string | null,
): string {
	const payload: IPersistedTabs = {
		v: PERSIST_VERSION,
		tabs,
		...(activeKey ? { active: activeKey } : {}),
	};
	return JSON.stringify(payload);
}

function isDocument(value: unknown): value is IEditorDocument {
	if (typeof value !== "object" || value === null) return false;
	const doc = value as Record<string, unknown>;
	const scoped = doc.scope === "app" || doc.scope === "user";
	switch (doc.kind) {
		case "board":
			return typeof doc.fileId === "string";
		case "page":
			return typeof doc.pageId === "string";
		case "widget":
			return typeof doc.widgetId === "string";
		case "storage":
			return scoped && typeof doc.location === "string";
		case "table":
			return scoped && typeof doc.table === "string";
		case "styles":
			return true;
		default:
			return false;
	}
}

/**
 * Restored tabs are only a *claim* that a document existed — the caller still prunes them
 * against what the app has now, so a stale entry costs one render, not a broken tab.
 */
export function deserializeTabs(raw: string | null | undefined): {
	tabs: IEditorTab[];
	activeKey: string | null;
} {
	const empty = { tabs: [] as IEditorTab[], activeKey: null };
	if (!raw) return empty;

	let parsed: unknown;
	try {
		parsed = JSON.parse(raw);
	} catch {
		return empty;
	}

	if (typeof parsed !== "object" || parsed === null) return empty;
	const payload = parsed as Record<string, unknown>;
	if (payload.v !== PERSIST_VERSION || !Array.isArray(payload.tabs)) {
		return empty;
	}

	const seen = new Set<string>();
	const tabs: IEditorTab[] = [];
	for (const entry of payload.tabs) {
		const tab = parseTab(entry);
		if (!tab || seen.has(tab.key)) continue;
		seen.add(tab.key);
		tabs.push(tab);
	}

	const active =
		typeof payload.active === "string" && seen.has(payload.active)
			? payload.active
			: null;
	return { tabs: withPinnedFirst(tabs), activeKey: active };
}

function parseTab(entry: unknown): IEditorTab | undefined {
	if (typeof entry !== "object" || entry === null) return undefined;
	const tab = entry as Record<string, unknown>;
	if (typeof tab.key !== "string" || !isDocument(tab.doc)) return undefined;
	return {
		key: tab.key,
		doc: tab.doc,
		...(typeof tab.layerPath === "string" ? { layerPath: tab.layerPath } : {}),
		...(tab.pinned === true ? { pinned: true } : {}),
		...(isTabColor(tab.color) ? { color: tab.color } : {}),
	};
}

function isTabColor(value: unknown): value is IEditorTabColor {
	return (EDITOR_TAB_COLORS as readonly unknown[]).includes(value);
}
