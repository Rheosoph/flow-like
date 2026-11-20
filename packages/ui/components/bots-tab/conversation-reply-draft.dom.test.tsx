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
	IBlockedPrincipal,
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

installDomGlobals();

const actualLocales = { ...(await import("@flow-like/locales")) };
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const actualNavigation = { ...(await import("next/navigation")) };
mock.module("next/navigation", () => ({
	...actualNavigation,
	useRouter: () => ({ push: () => {}, replace: () => {}, prefetch: () => {} }),
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
	"the-g": { id: "the-g", name: "The G", created_at: "" },
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
	beforeGet?: () => Promise<void>;
	getError?: Error;
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
	eventState: { alwaysRemote: () => false },
	boardState: {},
	inboxState: {
		blocks: [] as IBlockedPrincipal[],
		listBlocks: async () => backend.inboxState.blocks,
		blockPrincipal: async (principalId: string) => {
			backend.inboxState.blocks = [
				{
					principal_id: principalId,
					kind: users[principalId]?.bot ? "bot" : "user",
					created_at: "",
				},
			];
		},
		unblockPrincipal: async (principalId: string) => {
			backend.inboxState.blocks = backend.inboxState.blocks.filter(
				(block) => block.principal_id !== principalId,
			);
		},
		getConversation: async (id: string) => {
			const fake = roomOf(id);
			await fake.beforeGet?.();
			if (fake.getError) throw fake.getError;
			return fake.conversation;
		},
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
		uploads: [] as { conversationId: string; name: string }[],
		failUploads: false,
		uploadConversationFile: async (conversationId: string, file: File) => {
			if (backend.inboxState.failUploads) throw new Error("upload refused");
			backend.inboxState.uploads.push({ conversationId, name: file.name });
			return {
				file_id: "f1",
				name: file.name,
				app_id: `conversation_${conversationId}`,
				path: "f1.txt",
				bytes: file.size,
				sha256: "00",
				mime: file.type || "text/plain",
			};
		},
		conversationFileDownload: async (
			_id: string,
			handle: { mime: string },
		) => ({
			url: "https://files.test/f1",
			name: "note.txt",
			mime: handle.mime,
		}),
		markRead: async (id: string, upTo?: string) => {
			roomOf(id).marked.push(upTo);
			return { last_read_at: new Date().toISOString() };
		},
		renamed: [] as { conversationId: string; title: string }[],
		renameConversation: async (conversationId: string, title: string) => {
			backend.inboxState.renamed.push({ conversationId, title });
			const fake = roomOf(conversationId);
			fake.conversation = { ...fake.conversation, title: title || null };
			return fake.conversation;
		},
		setConversationImage: async (
			conversationId: string,
			image: string | null,
		) => {
			const fake = roomOf(conversationId);
			fake.conversation = { ...fake.conversation, image_url: image };
			return fake.conversation;
		},
	},
	bitState: { getProfileBits: async () => [] },
	userState: {
		getProfile: async () => null,
		getSettingsProfile: async () => null,
		getInfo: async () => null,
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

async function mountPanel(
	conversationId: string,
	me = "me",
	onClose = () => {},
): Promise<Panel> {
	const host = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(host as never);
	const panelRoot = createRoot(host);
	const panel = createElement(ConversationPanel, {
		conversationId,
		me,
		bots: [],
		onClose,
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
	mock.module("next/navigation", () => actualNavigation);
	mock.restore();
	mock.module("@flow-like/locales", () => actualLocales);
	restoreDomGlobals();
});

const { deliverRealtimeReply } = await import(
	"../../lib/realtime/reply-streams"
);

function pushed(
	text: string,
	state: "writing" | "working" | "done" | "dropped" = "writing",
	extra: { draft?: number; messageId?: string } = {},
) {
	deliverRealtimeReply({
		protocol_version: 1,
		kind: "reply",
		conversation_id: "c1",
		message_id: extra.messageId ?? "r-draft",
		reply_to: "a1",
		sender_id: "bot_scout",
		offset: 0,
		text,
		state,
		draft: extra.draft ?? 0,
	});
}

async function feed(...args: Parameters<typeof pushed>) {
	await act(async () => pushed(...args));
	await settle();
}

const count = (text: string, part: string) => text.split(part).length - 1;

describe("a reply a bot is still writing", () => {
	test("shows as a live bubble, grows, and is replaced by the stored message in place", async () => {
		await feed("Looking at the");
		expect(c1Panel.text()).toContain("Looking at the");
		const bubble = () =>
			c1Panel.host.querySelector('[data-chat-message="r-draft"]');
		expect(bubble()).not.toBeNull();

		await feed("Looking at the pricing pages now.");
		expect(c1Panel.text()).toContain("Looking at the pricing pages now.");
		expect(count(c1Panel.text(), "Looking at the pricing")).toBe(1);

		await feed("A rewrite.", "writing", { draft: 1 });
		expect(c1Panel.text()).toContain("A rewrite.");
		expect(c1Panel.text()).not.toContain("pricing pages");

		await feed("A rewrite.", "done", { draft: 1 });
		expect(bubble()).not.toBeNull();
		const before = bubble();

		c1.entries.unshift(
			entry("message", "bot_scout", { text: "A rewrite." }, 12, "r-draft"),
		);
		await pollNow();
		expect(count(c1Panel.text(), "A rewrite.")).toBe(1);
		expect(
			c1Panel.host.querySelectorAll('[data-chat-message="r-draft"]').length,
		).toBe(1);
		expect(before).not.toBeNull();
	});

	test("a dropped draft leaves no bubble and the bot is not announced twice", async () => {
		await feed("Half an answer", "writing", { messageId: "r-gone" });
		expect(c1Panel.text()).toContain("Half an answer");
		await feed("", "dropped", { messageId: "r-gone" });
		expect(c1Panel.text()).not.toContain("Half an answer");
	});
});
