import {
	afterAll,
	beforeAll,
	beforeEach,
	describe,
	expect,
	mock,
	spyOn,
	test,
} from "bun:test";
import { act, createElement } from "react";
import type {
	IConversation,
	IInboxEntriesQuery,
	IInboxEntry,
	ISendInboxEntryRequest,
	InboxEntryKind,
} from "../../state/backend-state/inbox-state";
import type { IUserLookup } from "../../state/backend-state/types";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	settle,
	translate,
	domWindow as window,
} from "./testing/dom-harness";

const actualLocales = { ...(await import("@flow-like/locales")) };
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const { createRoot } = await import("react-dom/client");
const { QueryClient, QueryClientProvider } = await import(
	"@tanstack/react-query"
);
const { toast } = await import("sonner");
const { useBackendStore } = await import("../../state/backend-state");
const { INBOX_POLL_MS, compareInboxCursors, inboxKeys } = await import(
	"../../hooks/use-inbox"
);
const { ConversationPanel } = await import("./conversation-panel");

const BASE = Date.UTC(2026, 10, 1, 9, 0, 0);
let sequence = 0;

function entry(
	kind: InboxEntryKind,
	sender: string,
	body: Record<string, unknown>,
	minute: number,
	messageId = `m${++sequence}`,
) {
	const time = BASE + minute * 60_000;
	return {
		id: `e_${messageId}`,
		message_id: messageId,
		conversation_id: "c1",
		sender_id: sender,
		kind,
		envelope: { v: 1, enc: "none", body },
		created_at: new Date(time).toISOString(),
		cursor: `${time * 1000}:e_${messageId}`,
	} satisfies IInboxEntry;
}

const conversation: IConversation = {
	id: "c1",
	kind: "group",
	title: "Q4 launch",
	encryption: "plain",
	created_by: "me",
	created_at: new Date(BASE).toISOString(),
	last_activity_at: new Date(BASE + 10 * 60_000).toISOString(),
	members: [
		{ principal_id: "me", kind: "user", added_by: null, joined_at: "" },
		{ principal_id: "anna", kind: "user", added_by: "me", joined_at: "" },
		{ principal_id: "bot_scout", kind: "bot", added_by: "anna", joined_at: "" },
	],
	unread_count: 2,
	last_entry: null,
};

const newestFirst = [
	entry("reaction", "anna", { target: "s2", emoji: "👍", op: "add" }, 9),
	entry("reaction", "me", { target: "s2", emoji: "👍", op: "add" }, 8),
	entry(
		"message",
		"bot_scout",
		{ text: "Two rivals changed pricing." },
		3,
		"s2",
	),
	entry("message", "bot_scout", { text: "I read 18 sources." }, 2, "s1"),
	entry(
		"member",
		"anna",
		{ op: "join", principal_id: "bot_scout", by: "anna" },
		1,
		"j1",
	),
	entry("message", "anna", { text: "Can someone check competitors?" }, 0, "a1"),
];

const users: Record<string, IUserLookup> = {
	me: { id: "me", name: "Felix Schultz", created_at: "" },
	anna: { id: "anna", name: "Anna Weber", created_at: "" },
	bot_scout: {
		id: "bot_scout",
		name: "Scout",
		created_at: "",
		bot: {
			owner_id: "anna",
			system: null,
			appearance: {} as never,
			deleted: false,
		},
	},
};

/** One conversation on the fake server: entries in any order, paged like the real routes. */
interface FakeRoom {
	conversation: IConversation;
	entries: IInboxEntry[];
	queries: IInboxEntriesQuery[];
	sent: ISendInboxEntryRequest[];
	marked: (string | undefined)[];
	minute: number;
	beforeList?: (query: IInboxEntriesQuery) => Promise<void>;
	beforeSend?: (request: ISendInboxEntryRequest) => Promise<void>;
}

const rooms = new Map<string, FakeRoom>();

function room(
	id: string,
	entries: IInboxEntry[],
	overrides: Partial<IConversation> = {},
): FakeRoom {
	const fake: FakeRoom = {
		conversation: { ...conversation, id, ...overrides },
		entries: entries.map((item) => ({ ...item, conversation_id: id })),
		queries: [],
		sent: [],
		marked: [],
		minute: 1_000,
	};
	rooms.set(id, fake);
	return fake;
}

function roomOf(id: string): FakeRoom {
	const fake = rooms.get(id);
	if (!fake) throw new Error(`No fake conversation ${id}`);
	return fake;
}

function pageOf(entries: readonly IInboxEntry[], query: IInboxEntriesQuery) {
	const limit = query.limit ?? 50;
	const { before, after } = query;
	const matching = [...entries]
		.sort((a, b) => compareInboxCursors(b.cursor, a.cursor))
		.filter(
			(item) =>
				(!before || compareInboxCursors(item.cursor, before) < 0) &&
				(!after || compareInboxCursors(item.cursor, after) > 0),
		);
	const page = matching.slice(0, limit);
	return {
		entries: page,
		next_before:
			matching.length > limit ? (page[page.length - 1]?.cursor ?? null) : null,
	};
}

const backend = {
	inboxState: {
		getConversation: async (id: string) => roomOf(id).conversation,
		listEntries: async (id: string, query: IInboxEntriesQuery = {}) => {
			const fake = roomOf(id);
			fake.queries.push(query);
			await fake.beforeList?.(query);
			return pageOf(fake.entries, query);
		},
		sendEntry: async (id: string, request: ISendInboxEntryRequest) => {
			const fake = roomOf(id);
			fake.sent.push(request);
			await fake.beforeSend?.(request);
			const known = fake.entries.find(
				(item) => item.message_id === request.message_id,
			);
			if (known) return known;
			const body = (request.envelope?.body ?? {}) as Record<string, unknown>;
			const sent = {
				...entry(request.kind, "me", body, fake.minute++, request.message_id),
				conversation_id: id,
			};
			fake.entries.push(sent);
			return sent;
		},
		markRead: async (id: string, upTo?: string) => {
			roomOf(id).marked.push(upTo);
			return { last_read_at: new Date().toISOString() };
		},
	},
	userState: {
		lookupUser: async (id: string) => users[id] ?? null,
		lookupUsers: async (ids: string[]) =>
			ids.map((id) => users[id]).filter(Boolean),
	},
};

/** Takes the tail poll's timer off the clock; tests run its ticks by hand. */
function capturePollTimers() {
	const ticks = new Map<number, () => unknown>();
	const descriptors = (["setInterval", "clearInterval"] as const).map(
		(key) => [key, Object.getOwnPropertyDescriptor(window, key)] as const,
	);
	const realSetInterval = window.setInterval.bind(window);
	const realClearInterval = window.clearInterval.bind(window);
	let next = 0;
	Object.assign(window, {
		setInterval: (handler: () => unknown, ms?: number, ...args: unknown[]) => {
			if (ms !== INBOX_POLL_MS.openConversation) {
				return realSetInterval(handler, ms, ...args);
			}
			next -= 1;
			ticks.set(next, handler);
			return next;
		},
		clearInterval: (id: unknown) => {
			if (!ticks.delete(id as number)) realClearInterval(id as never);
		},
	});
	const restore = () => {
		for (const [key, descriptor] of descriptors) {
			if (descriptor) Object.defineProperty(window, key, descriptor);
		}
	};
	return { ticks, restore };
}

const pollTimers = capturePollTimers();

function startPolls() {
	return Promise.all([...pollTimers.ticks.values()].map((tick) => tick()));
}

/** Runs one tick of every open conversation's tail poll. */
async function pollNow() {
	await act(async () => {
		await startPolls();
	});
	await settle();
}

const previousBackend = useBackendStore.getState().backend;
const client = new QueryClient({
	defaultOptions: { queries: { retry: false } },
});
const toastError = spyOn(toast, "error").mockImplementation(() => 0);

interface Panel {
	host: HTMLElement;
	text: () => string;
	unmount: () => Promise<void>;
}

async function mountPanel(conversationId: string): Promise<Panel> {
	const host = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(host as never);
	const panelRoot = createRoot(host);
	const panel = createElement(ConversationPanel, {
		conversationId,
		me: "me",
		bots: [],
		onClose: () => {},
	});
	await act(async () =>
		panelRoot.render(createElement(QueryClientProvider, { client }, panel)),
	);
	await settle();
	await settle(100);
	return {
		host,
		text: () => host.textContent ?? "",
		unmount: async () => {
			await act(async () => panelRoot.unmount());
			host.remove();
		},
	};
}

function button(panel: Panel, label: string) {
	return [...panel.host.querySelectorAll("button")].find(
		(item) =>
			item.textContent === label || item.getAttribute("aria-label") === label,
	);
}

async function click(element: Element | undefined) {
	expect(element).toBeDefined();
	await act(async () => (element as HTMLElement).click());
	await settle();
}

async function setText(input: Element | null, text: string) {
	const setValue = Object.getOwnPropertyDescriptor(
		window.HTMLTextAreaElement.prototype,
		"value",
	)?.set;
	await act(async () => {
		setValue?.call(input, text);
		input?.dispatchEvent(new window.Event("input", { bubbles: true }) as never);
	});
}

async function typeAndSubmit(panel: Panel, text: string) {
	await setText(panel.host.querySelector("form textarea"), text);
	const form = panel.host.querySelector("form") as HTMLFormElement;
	const submit = new window.Event("submit", {
		bubbles: true,
		cancelable: true,
	});
	await act(async () => {
		form.dispatchEvent(submit as never);
	});
	await settle();
}

async function chooseMenuItem(trigger: Element | undefined, label: string) {
	expect(trigger).toBeDefined();
	await act(async () => {
		trigger?.dispatchEvent(
			new window.PointerEvent("pointerdown", {
				bubbles: true,
				button: 0,
			}) as never,
		);
	});
	await settle();
	const item = [...window.document.querySelectorAll("[role='menuitem']")].find(
		(node) => node.textContent === label,
	);
	await click(item as unknown as Element | undefined);
}

const c1 = room("c1", newestFirst);
c1.minute = 11;
let c1Panel: Panel;

beforeAll(async () => {
	installDomGlobals();
	useBackendStore.getState().setBackend(backend as never);
	c1Panel = await mountPanel("c1");
});

beforeEach(() => {
	toastError.mockClear();
});

afterAll(async () => {
	await c1Panel.unmount();
	client.clear();
	useBackendStore.setState({ backend: previousBackend });
	await new Promise((resolve) => setTimeout(resolve, 0));
	pollTimers.restore();
	mock.restore();
	mock.module("@flow-like/locales", () => actualLocales);
	restoreDomGlobals();
});

describe("ConversationPanel", () => {
	test("shows the room with folded, grouped messages", () => {
		const text = c1Panel.text();
		expect(text).toContain("Q4 launch");
		expect(text).toContain("Group · 2 people, 1 bot");
		expect(text).toContain("Anna Weber added Scout");
		expect(text).toContain("I read 18 sources.");
		expect(text).toContain("Two rivals changed pricing.");
		expect(text.split("Scout").length - 1).toBe(2);
		expect(text).toContain("Anna's bot");
	});

	test("aggregates reactions and marks mine", () => {
		const chip = c1Panel.host.querySelector(
			"button[aria-pressed='true'][aria-label^='👍']",
		);
		expect(chip?.textContent).toBe("👍2");
		expect(chip?.getAttribute("aria-label")).toBe("👍 from You, Anna Weber");
	});

	test("marks the newest entry as read", () => {
		expect(c1.marked).toEqual([newestFirst[0]?.cursor]);
	});

	test("sends a plain message envelope", async () => {
		await typeAndSubmit(c1Panel, "Thanks, Scout!");
		expect(c1.sent).toHaveLength(1);
		expect(c1.sent[0]?.kind).toBe("message");
		expect(c1.sent[0]?.message_id).toMatch(/^msg_[0-9a-f]{32}$/);
		expect(c1.sent[0]?.envelope).toEqual({
			v: 1,
			enc: "none",
			body: { text: "Thanks, Scout!" },
		});
		expect(c1Panel.text()).toContain("Thanks, Scout!");
		expect(c1Panel.text()).not.toContain("Sending…");
	});
});

describe("live updates", () => {
	test("a send doesn't move the poll past entries it hasn't fetched", async () => {
		const fake = room("tail", [
			entry("message", "anna", { text: "Lunch?" }, 0),
		]);
		const panel = await mountPanel("tail");
		const tail = fake.entries[0]?.cursor;
		fake.entries.push(entry("message", "anna", { text: "Pizza or sushi?" }, 1));
		await typeAndSubmit(panel, "Sushi!");
		await pollNow();
		expect(fake.queries.at(-1)?.after).toBe(tail);
		expect(panel.text()).toContain("Pizza or sushi?");
		expect(panel.text()).toContain("Sushi!");
		await panel.unmount();
	});

	test("a send while a poll is in flight doesn't drop that poll", async () => {
		const fake = room("inflight", [
			entry("message", "anna", { text: "Ready?" }, 0),
		]);
		const panel = await mountPanel("inflight");
		fake.entries.push(entry("message", "anna", { text: "Starting now" }, 1));
		let release = () => {};
		fake.beforeList = (query) =>
			query.after
				? new Promise<void>((resolve) => {
						release = resolve;
					})
				: Promise.resolve();
		let polls: Promise<unknown> = Promise.resolve();
		await act(async () => {
			polls = startPolls();
		});
		await typeAndSubmit(panel, "Go");
		fake.beforeList = undefined;
		await act(async () => {
			release();
			await polls;
		});
		await settle();
		expect(panel.text()).toContain("Starting now");
		await panel.unmount();
	});

	test("a burst larger than one page reloads the newest page instead of leaving a gap", async () => {
		const fake = room("burst", [entry("message", "anna", { text: "old" }, 0)]);
		const panel = await mountPanel("burst");
		const tail = fake.entries[0]?.cursor;
		for (let minute = 1; minute <= 60; minute++) {
			fake.entries.push(
				entry("message", "anna", { text: `burst ${minute}` }, minute),
			);
		}
		const seen = fake.queries.length;
		await pollNow();
		expect(fake.queries.slice(seen)).toEqual([
			{ as: undefined, after: tail, limit: 50 },
			{ as: undefined, before: undefined, limit: 50 },
		]);
		expect(panel.text()).toContain("burst 60");
		expect(panel.text()).not.toContain("burst 10");
		await click(button(panel, "Load older messages"));
		expect(panel.text()).toContain("burst 10");
		expect(panel.text()).toContain("old");
		await panel.unmount();
	});

	test("a failed 'Load older' doesn't stop live updates", async () => {
		const history = Array.from({ length: 55 }, (_, minute) =>
			entry("message", "anna", { text: `history ${minute}` }, minute),
		);
		const fake = room("gate", history);
		const panel = await mountPanel("gate");
		fake.beforeList = async (query) => {
			if (query.before) throw new Error("offline");
		};
		await click(button(panel, "Load older messages"));
		fake.beforeList = undefined;
		fake.entries.push(entry("message", "anna", { text: "Still here" }, 100));
		await pollNow();
		expect(panel.text()).toContain("Still here");
		await panel.unmount();
	});
});

describe("marking read", () => {
	test("the reader's own entries never mark read, and the open conversation's badge clears", async () => {
		const fake = room(
			"read-own",
			[entry("message", "anna", { text: "Ping" }, 0)],
			{ unread_count: 2 },
		);
		const panel = await mountPanel("read-own");
		expect(fake.marked).toEqual([fake.entries[0]?.cursor]);
		expect(
			client.getQueryData<IConversation>(inboxKeys.conversation("read-own"))
				?.unread_count,
		).toBe(0);
		await typeAndSubmit(panel, "Pong");
		await pollNow();
		expect(fake.marked).toHaveLength(1);
		await panel.unmount();
	});

	test("a batch from others is marked read even when its newest entry is the reader's", async () => {
		const fake = room(
			"read-batch",
			[entry("message", "anna", { text: "Hi" }, 0)],
			{ unread_count: 0 },
		);
		const panel = await mountPanel("read-batch");
		const opened = fake.entries[0]?.cursor;
		fake.entries.push(entry("message", "anna", { text: "Are you there?" }, 1));
		const mine = entry("message", "me", { text: "Yes, from my phone" }, 2);
		fake.entries.push(mine);
		await pollNow();
		expect(panel.text()).toContain("Are you there?");
		expect(fake.marked).toEqual([opened, mine.cursor]);
		await panel.unmount();
	});
});

describe("sending", () => {
	test("a send that fails after the view closed comes back as failed, with retry", async () => {
		const fake = room(
			"away",
			[entry("message", "anna", { text: "Call me" }, 0)],
			{ title: "Weekend" },
		);
		let fail = (_error: Error) => {};
		fake.beforeSend = () =>
			new Promise<void>((_, reject) => {
				fail = reject;
			});
		let panel = await mountPanel("away");
		await typeAndSubmit(panel, "On my way");
		expect(panel.text()).toContain("Sending…");
		await panel.unmount();
		await act(async () => fail(new Error("offline")));
		await settle();
		expect(toastError.mock.calls).toEqual([
			["Not sent", { description: "Weekend" }],
		]);

		fake.beforeSend = undefined;
		panel = await mountPanel("away");
		expect(panel.text()).toContain("On my way");
		expect(panel.text()).toContain("Not sent");
		await click(button(panel, "Retry"));
		const first = fake.sent[0]?.message_id;
		expect(fake.sent.map((request) => request.message_id)).toEqual([
			first,
			first,
		]);
		expect(panel.text()).toContain("On my way");
		expect(panel.text()).not.toContain("Not sent");
		expect(toastError).toHaveBeenCalledTimes(1);
		await panel.unmount();
	});

	test("a send that fails while the view is open shows in place, without a toast", async () => {
		const fake = room("here", [entry("message", "anna", { text: "Hey" }, 0)]);
		fake.beforeSend = async () => {
			throw new Error("offline");
		};
		const panel = await mountPanel("here");
		await typeAndSubmit(panel, "Hello?");
		expect(panel.text()).toContain("Not sent");
		expect(toastError).not.toHaveBeenCalled();
		await click(button(panel, "Discard"));
		expect(panel.text()).not.toContain("Hello?");
		await panel.unmount();
	});

	test("a failed edit keeps the editor open with the new text", async () => {
		const fake = room("edit", [
			entry("message", "me", { text: "Meet at 5" }, 0, "mine"),
		]);
		const panel = await mountPanel("edit");
		await chooseMenuItem(button(panel, "More actions"), "Edit");
		const editor = () =>
			panel.host.querySelector<HTMLTextAreaElement>(
				"textarea[aria-label='Edit message']",
			);
		await setText(editor(), "Meet at 6");
		fake.beforeSend = async () => {
			throw new Error("offline");
		};
		await click(button(panel, "Save"));
		expect(fake.sent.at(-1)?.kind).toBe("edit");
		expect(toastError).toHaveBeenCalledWith(
			"That didn't go through. Try again.",
		);
		expect(editor()?.value).toBe("Meet at 6");
		expect(panel.text()).not.toContain("(edited)");
		await panel.unmount();
	});
});

describe("history", () => {
	test("loads an older page on its own when the newest one holds no message", async () => {
		const reactions = Array.from({ length: 50 }, (_, index) =>
			entry(
				"reaction",
				`member_${index}`,
				{ target: "announce", emoji: "🎉", op: "add" },
				10 + index,
			),
		);
		const fake = room("reactions", [
			entry("message", "anna", { text: "We shipped!" }, 0, "announce"),
			...reactions,
		]);
		const panel = await mountPanel("reactions");
		expect(panel.text()).toContain("We shipped!");
		expect(panel.text()).not.toContain("No messages yet");
		expect(fake.queries.filter((query) => query.before)).toHaveLength(1);
		await panel.unmount();
	});

	test("stops after a few pages and offers older messages instead of an empty state", async () => {
		const reactions = Array.from({ length: 300 }, (_, index) =>
			entry(
				"reaction",
				"anna",
				{ target: "ancient", emoji: "👍", op: "add" },
				10 + index,
			),
		);
		const fake = room("only-reactions", [
			entry("message", "anna", { text: "Ancient news" }, 0, "ancient"),
			...reactions,
		]);
		const panel = await mountPanel("only-reactions");
		await settle(100);
		expect(fake.queries.filter((query) => query.before)).toHaveLength(5);
		expect(panel.text()).toContain("Load older messages");
		expect(panel.text()).not.toContain("No messages yet");
		expect(panel.text()).not.toContain("Ancient news");
		await panel.unmount();
	});
});
