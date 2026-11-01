import { create } from "zustand";

export type ChatActivityTarget =
	| { kind: "flowpilot"; sessionId: string }
	| { kind: "app"; sessionId: string; appId: string; eventId?: string };

export interface ChatActivityEntry {
	id: string;
	target: ChatActivityTarget;
	messageId: string;
	title: string;
	preview: string;
	createdAt: number;
}

interface ChatActivityState {
	/** One unread entry per conversation, newest reply first. */
	entries: ChatActivityEntry[];
	markRead: (id: string) => void;
	clear: () => void;
}

type ActivityStorage = Pick<Storage, "getItem" | "setItem">;
const STORAGE_KEY = "flow-like:chat-activity:v1";
const MAX_ENTRIES = 100;
const MAX_RECEIPTS = 1000;

export function chatActivityKey(target: ChatActivityTarget): string {
	return JSON.stringify([
		target.kind,
		target.kind === "app" ? target.appId : "",
		target.kind === "app" ? (target.eventId ?? "") : "",
		target.sessionId,
	]);
}

function compact(value: string, limit: number): string {
	return value.replace(/\s+/g, " ").trim().slice(0, limit);
}

function readEntry(value: unknown): ChatActivityEntry | undefined {
	if (!value || typeof value !== "object") return;
	const entry = value as Partial<ChatActivityEntry>;
	const target = entry.target;
	if (
		!target ||
		typeof target.sessionId !== "string" ||
		!target.sessionId ||
		(target.kind !== "flowpilot" && target.kind !== "app") ||
		(target.kind === "app" &&
			(typeof target.appId !== "string" ||
				!target.appId ||
				(target.eventId !== undefined &&
					typeof target.eventId !== "string"))) ||
		typeof entry.messageId !== "string" ||
		!entry.messageId ||
		typeof entry.title !== "string" ||
		typeof entry.preview !== "string" ||
		typeof entry.createdAt !== "number" ||
		!Number.isFinite(entry.createdAt) ||
		entry.createdAt < 0 ||
		entry.createdAt > 8.64e15
	) {
		return;
	}
	return {
		id: chatActivityKey(target),
		target,
		messageId: entry.messageId,
		title: compact(entry.title, 120),
		preview: compact(entry.preview, 200),
		createdAt: entry.createdAt,
	};
}

/** Completion receipts are separate from unread entries, so reading a reply cannot make replay notify again. */
export function createChatActivityTracker(options: {
	storage: () => ActivityStorage | undefined;
	isForeground: () => boolean;
}) {
	let initialized = false;
	let lastStoredValue: string | null | undefined;
	const receipts = new Set<string>();
	const surfaces = new Map<symbol, string>();
	const listeners = new Set<(entry: ChatActivityEntry) => void>();

	const store = create<ChatActivityState>(() => ({
		entries: [],
		markRead: (id) => {
			initialize(true);
			const entries = store.getState().entries;
			if (!entries.some((entry) => entry.id === id)) return;
			store.setState({ entries: entries.filter((entry) => entry.id !== id) });
			persist();
		},
		clear: () => {
			initialize(true);
			store.setState({ entries: [] });
			persist();
		},
	}));

	function persist() {
		try {
			const storage = options.storage();
			if (!storage) return;
			const serialized = JSON.stringify({
				entries: store.getState().entries,
				receipts: [...receipts],
			});
			storage.setItem(STORAGE_KEY, serialized);
			lastStoredValue = serialized;
		} catch {
			// Replies still surface when browser storage is unavailable or full.
		}
	}

	function initialize(refresh = false) {
		if (initialized && !refresh) return;
		try {
			const storage = options.storage();
			if (!storage) return;
			initialized = true;
			const raw = storage.getItem(STORAGE_KEY);
			if (raw === lastStoredValue) return;
			lastStoredValue = raw;
			if (!raw) {
				receipts.clear();
				if (store.getState().entries.length) store.setState({ entries: [] });
				return;
			}
			const saved = JSON.parse(raw);
			if (!saved || typeof saved !== "object") return;
			receipts.clear();
			if (Array.isArray(saved.receipts)) {
				for (const receipt of saved.receipts.slice(-MAX_RECEIPTS)) {
					if (typeof receipt === "string") receipts.add(receipt);
				}
			}
			if (Array.isArray(saved.entries)) {
				const ids = new Set<string>();
				const entries = saved.entries
					.map(readEntry)
					.filter(
						(
							entry: ChatActivityEntry | undefined,
						): entry is ChatActivityEntry => {
							if (!entry || ids.has(entry.id)) return false;
							ids.add(entry.id);
							return true;
						},
					)
					.sort(
						(a: ChatActivityEntry, b: ChatActivityEntry) =>
							b.createdAt - a.createdAt,
					)
					.slice(0, MAX_ENTRIES);
				store.setState({ entries });
			}
		} catch {
			// Ignore corrupt or inaccessible browser storage.
			initialized = true;
		}
	}

	function refreshVisibility() {
		initialize(true);
		if (!options.isForeground()) return;
		const visible = new Set(surfaces.values());
		const entries = store.getState().entries;
		const unread = entries.filter((entry) => !visible.has(entry.id));
		if (unread.length === entries.length) return;
		store.setState({ entries: unread });
		persist();
	}

	function record(input: Omit<ChatActivityEntry, "id" | "createdAt">) {
		// Another window may have received or read a reply since this store last changed.
		initialize(true);
		const id = chatActivityKey(input.target);
		const receipt = JSON.stringify([id, input.messageId]);
		if (receipts.has(receipt)) return;
		receipts.add(receipt);
		while (receipts.size > MAX_RECEIPTS) {
			const oldest = receipts.values().next().value;
			if (oldest !== undefined) receipts.delete(oldest);
		}
		if (options.isForeground() && [...surfaces.values()].includes(id)) {
			refreshVisibility();
			persist();
			return;
		}
		const entry: ChatActivityEntry = {
			...input,
			id,
			title: compact(input.title, 120),
			preview: compact(input.preview, 200),
			createdAt: Date.now(),
		};
		store.setState({
			entries: [
				entry,
				...store.getState().entries.filter((item) => item.id !== id),
			].slice(0, MAX_ENTRIES),
		});
		persist();
		for (const listener of listeners) {
			try {
				listener(entry);
			} catch {
				// Notification presentation must not interrupt a completed chat run.
			}
		}
		return entry;
	}

	function registerSurface(target: ChatActivityTarget) {
		const token = Symbol();
		surfaces.set(token, chatActivityKey(target));
		refreshVisibility();
		return () => {
			surfaces.delete(token);
		};
	}

	return {
		store,
		initialize,
		synchronize: refreshVisibility,
		record,
		refreshVisibility,
		registerSurface,
		remove: (target: ChatActivityTarget) =>
			store.getState().markRead(chatActivityKey(target)),
		subscribe: (listener: (entry: ChatActivityEntry) => void) => {
			listeners.add(listener);
			return () => {
				listeners.delete(listener);
			};
		},
	};
}

const tracker = createChatActivityTracker({
	storage: () =>
		typeof window === "undefined" ? undefined : window.localStorage,
	isForeground: () =>
		typeof document !== "undefined" &&
		document.visibilityState === "visible" &&
		document.hasFocus(),
});

export const useChatActivityStore = tracker.store;
let shellEnabled = false;

/** Only shells that render the inbox retain activity. Embedded and locked service UIs have no inbox. */
export function initializeChatActivity() {
	shellEnabled = true;
	tracker.initialize();
}

export const syncChatActivity = tracker.synchronize;
export function recordChatActivity(
	input: Parameters<typeof tracker.record>[0],
) {
	if (!shellEnabled) return;
	return tracker.record(input);
}
export const removeChatActivity = tracker.remove;
export const subscribeChatActivity = tracker.subscribe;
export const registerChatActivitySurface = tracker.registerSurface;
export const refreshChatActivityVisibility = tracker.refreshVisibility;
