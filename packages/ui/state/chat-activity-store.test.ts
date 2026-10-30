import { describe, expect, test } from "bun:test";
import {
	type ChatActivityTarget,
	chatActivityKey,
	createChatActivityTracker,
} from "./chat-activity-store";

const first: ChatActivityTarget = { kind: "flowpilot", sessionId: "first" };
const second: ChatActivityTarget = { kind: "flowpilot", sessionId: "second" };

function harness() {
	const saved = new Map<string, string>();
	const storage = {
		getItem: (key: string) => saved.get(key) ?? null,
		setItem: (key: string, value: string) => {
			saved.set(key, value);
		},
	};
	let focused = true;
	const makeTracker = () =>
		createChatActivityTracker({
			storage: () => storage,
			isForeground: () => focused,
		});
	return {
		tracker: makeTracker(),
		makeTracker,
		storage,
		saved,
		focus: (value: boolean) => {
			focused = value;
		},
	};
}

function reply(target = first, messageId = "reply-1") {
	return {
		target,
		messageId,
		title: "Weekly report",
		preview: "The report is ready.",
	};
}

describe("chat activity", () => {
	test("keeps one unread entry per conversation and announces new replies once", () => {
		const { tracker } = harness();
		const popups: string[] = [];
		tracker.subscribe((entry) => popups.push(entry.messageId));
		tracker.record(reply());
		tracker.record(reply(second, "reply-2"));
		tracker.record(reply(first, "reply-3"));
		tracker.record(reply(first, "reply-3"));
		expect(
			tracker.store.getState().entries.map((entry) => entry.messageId),
		).toEqual(["reply-3", "reply-2"]);
		expect(popups).toEqual(["reply-1", "reply-2", "reply-3"]);
	});

	test("suppresses only the conversation actually displayed in a focused surface", () => {
		const { tracker } = harness();
		const release = tracker.registerSurface(first);
		expect(tracker.record(reply())).toBeUndefined();
		tracker.record(reply(second));
		expect(
			tracker.store.getState().entries.map((entry) => entry.target),
		).toEqual([second]);
		release();
		tracker.record(reply(first, "after-close"));
		expect(tracker.store.getState().entries).toHaveLength(2);
	});

	test("retains hidden-window replies until focus returns to their conversation", () => {
		const { tracker, focus } = harness();
		tracker.registerSurface(first);
		focus(false);
		tracker.record(reply());
		tracker.record(reply(second));
		tracker.refreshVisibility();
		expect(tracker.store.getState().entries).toHaveLength(2);
		focus(true);
		tracker.refreshVisibility();
		expect(
			tracker.store.getState().entries.map((entry) => entry.target),
		).toEqual([second]);
	});

	test("opening a conversation clears its unread entry without enabling replay", () => {
		const { tracker } = harness();
		tracker.record(reply());
		const release = tracker.registerSurface(first);
		expect(tracker.store.getState().entries).toHaveLength(0);
		release();
		expect(tracker.record(reply())).toBeUndefined();
		expect(tracker.store.getState().entries).toHaveLength(0);
	});

	test("keeps surface registration until every mounted copy has left", () => {
		const { tracker } = harness();
		const releaseFirst = tracker.registerSurface(first);
		const releaseSecond = tracker.registerSurface(first);
		releaseFirst();
		releaseFirst();
		expect(tracker.record(reply())).toBeUndefined();
		releaseSecond();
		expect(tracker.record(reply(first, "new-reply"))).toBeDefined();
	});

	test("restores unread entries without replaying popups and remembers read receipts", () => {
		const { tracker, makeTracker } = harness();
		tracker.record(reply());
		tracker.record(reply(second, "second-reply"));
		tracker.store.getState().markRead(chatActivityKey(first));
		const restored = makeTracker();
		const popups: string[] = [];
		restored.subscribe((entry) => popups.push(entry.messageId));
		restored.initialize();
		expect(
			restored.store.getState().entries.map((entry) => entry.target),
		).toEqual([second]);
		restored.record(reply());
		restored.record(reply(second, "second-reply"));
		expect(popups).toEqual([]);
		expect(restored.store.getState().entries).toHaveLength(1);
	});

	test("hydrates before recording a reply that finishes ahead of the provider mount", () => {
		const { tracker, makeTracker } = harness();
		tracker.record(reply());
		const reloaded = makeTracker();
		reloaded.record(reply(second, "new"));
		expect(
			reloaded.store.getState().entries.map((entry) => entry.target),
		).toEqual([second, first]);
	});

	test("preserves another window's replies and synchronizes read state without popups", () => {
		const { tracker, makeTracker } = harness();
		const otherWindow = makeTracker();
		tracker.initialize();
		otherWindow.initialize();
		tracker.record(reply());
		otherWindow.record(reply(second));
		expect(otherWindow.store.getState().entries).toHaveLength(2);
		tracker.synchronize();
		expect(tracker.store.getState().entries).toHaveLength(2);
		const popups: string[] = [];
		otherWindow.subscribe((entry) => popups.push(entry.id));
		tracker.store.getState().markRead(chatActivityKey(first));
		otherWindow.synchronize();
		expect(
			otherWindow.store.getState().entries.map((entry) => entry.target),
		).toEqual([second]);
		expect(popups).toEqual([]);
		expect(otherWindow.record(reply())).toBeUndefined();
	});

	test("distinguishes app conversations, events, and FlowPilot with the same session id", () => {
		const { tracker } = harness();
		tracker.registerSurface(first);
		const app: ChatActivityTarget = {
			kind: "app",
			appId: "app-one",
			eventId: "chat",
			sessionId: first.sessionId,
		};
		tracker.record(reply(app));
		tracker.record(reply({ ...app, appId: "app-two" }));
		tracker.record(reply({ ...app, eventId: "other" }));
		expect(tracker.store.getState().entries).toHaveLength(3);
	});

	test("removing and clearing unread entries retain completion deduplication", () => {
		const { tracker } = harness();
		tracker.record(reply());
		tracker.record(reply(second));
		tracker.remove(first);
		expect(tracker.store.getState().entries).toHaveLength(1);
		tracker.store.getState().clear();
		tracker.record(reply());
		tracker.record(reply(second));
		expect(tracker.store.getState().entries).toHaveLength(0);
	});

	test("survives corrupt storage, blocked persistence, and a failing popup listener", () => {
		const tracker = createChatActivityTracker({
			storage: () => ({
				getItem: () => "broken json",
				setItem: () => {
					throw new Error("full");
				},
			}),
			isForeground: () => false,
		});
		tracker.subscribe(() => {
			throw new Error("popup failed");
		});
		expect(() => tracker.record(reply())).not.toThrow();
		expect(tracker.store.getState().entries).toHaveLength(1);
	});

	test("ignores malformed saved targets and bounds long previews", () => {
		const { tracker, saved, makeTracker } = harness();
		tracker.record({
			...reply(),
			title: "a".repeat(150),
			preview: `  ${"b".repeat(250)}\n  `,
		});
		const [key, raw] = [...saved][0];
		const data = JSON.parse(raw);
		data.entries.push({
			...data.entries[0],
			target: { kind: "app", sessionId: "x" },
		});
		saved.set(key, JSON.stringify(data));
		const restored = makeTracker();
		restored.initialize();
		const entries = restored.store.getState().entries;
		expect(entries).toHaveLength(1);
		expect(entries[0].title).toHaveLength(120);
		expect(entries[0].preview).toHaveLength(200);
	});
});
