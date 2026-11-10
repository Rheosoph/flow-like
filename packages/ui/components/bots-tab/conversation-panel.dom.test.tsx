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
import { ApiResponseError } from "../../lib/api-error";
import {
	type ConversationFeedListener,
	registerConversationFeed,
} from "../../lib/realtime/conversation-feed";
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
		expect(chip?.textContent?.startsWith("👍2")).toBe(true);
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

describe("composer", () => {
	const suggestionsOf = (panel: Panel) => [
		...panel.host.querySelectorAll("button[aria-current]"),
	];
	const caretText = async (panel: Panel, text: string) => {
		const input = panel.host.querySelector(
			"form textarea",
		) as unknown as HTMLTextAreaElement;
		await setText(input, text);
		await act(async () => {
			input.setSelectionRange(text.length, text.length);
			input.dispatchEvent(
				new window.Event("input", { bubbles: true }) as never,
			);
		});
	};
	const pick = async (panel: Panel) => {
		await act(async () => {
			(suggestionsOf(panel)[0] as unknown as HTMLElement).dispatchEvent(
				new window.MouseEvent("mousedown", {
					bubbles: true,
					cancelable: true,
				}) as never,
			);
		});
	};
	const submit = async (panel: Panel) => {
		await act(async () => {
			panel.host.querySelector("form")?.dispatchEvent(
				new window.Event("submit", {
					bubbles: true,
					cancelable: true,
				}) as never,
			);
		});
		await settle();
	};
	const sentBodies = (fake: FakeRoom) =>
		fake.sent
			.filter((item) => item.kind === "message")
			.map((item) => item.envelope?.body);

	test("typing @ lists the room's members and narrows", async () => {
		room("cp1", [entry("message", "anna", { text: "hi" }, 0)]);
		const panel = await mountPanel("cp1");
		await caretText(panel, "hi @");
		expect(suggestionsOf(panel).map((b) => b.textContent)).toEqual([
			"AWAnna Weber",
			"Scout",
		]);
		await caretText(panel, "hi @sc");
		expect(suggestionsOf(panel).map((b) => b.textContent)).toEqual(["Scout"]);
		await panel.unmount();
	});

	test("picking inserts the name and the send carries the mention", async () => {
		const fake = room("cp2", [entry("message", "anna", { text: "hi" }, 0)]);
		const panel = await mountPanel("cp2");
		await caretText(panel, "hi @sc");
		await pick(panel);
		expect(
			(panel.host.querySelector("form textarea") as HTMLTextAreaElement).value,
		).toBe("hi @Scout ");
		await caretText(panel, "hi @Scout do it");
		await submit(panel);
		expect(sentBodies(fake)).toEqual([
			{ text: "hi @Scout do it", mentions: ["bot_scout"] },
		]);
		await panel.unmount();
	});

	test("a typed-out @name counts, a longer word doesn't, a deleted one is dropped", async () => {
		const fake = room("cp3", [entry("message", "anna", { text: "hi" }, 0)]);
		const panel = await mountPanel("cp3");
		await caretText(panel, "@Scout and @Scoutx");
		await submit(panel);
		await caretText(panel, "@a");
		await pick(panel);
		await caretText(panel, "never mind");
		await submit(panel);
		expect(sentBodies(fake)).toEqual([
			{ text: "@Scout and @Scoutx", mentions: ["bot_scout"] },
			{ text: "never mind" },
		]);
		await panel.unmount();
	});

	test("the composer is the chat's own, with the hint in its placeholder and no line above it", async () => {
		room("cp4", [entry("message", "anna", { text: "hi" }, 0)]);
		const panel = await mountPanel("cp4");
		const field = () =>
			panel.host.querySelector("form textarea") as HTMLTextAreaElement;
		expect(field().placeholder).toBe(
			"Message Q4 launch · @mention a bot to make sure it answers",
		);
		expect(panel.text()).not.toContain("read along");
		await panel.unmount();
		room("cp5", [entry("message", "anna", { text: "hi" }, 0)], {
			members: conversation.members.filter((m) => m.kind === "user"),
		});
		const plain = await mountPanel("cp5");
		expect(
			(plain.host.querySelector("form textarea") as HTMLTextAreaElement)
				.placeholder,
		).toBe("Message Q4 launch");
		await plain.unmount();
	});
});

describe("attachments", () => {
	const pasteFile = async (panel: Panel, name: string) => {
		const file = new window.File(["hello"], name, { type: "text/plain" });
		const input = panel.host.querySelector("form textarea") as HTMLElement;
		const event = new window.Event("paste", {
			bubbles: true,
			cancelable: true,
		});
		Object.defineProperty(event, "clipboardData", {
			value: { items: [{ kind: "file", getAsFile: () => file }] },
		});
		await act(async () => {
			input.dispatchEvent(event as never);
		});
	};
	test("a pasted file is uploaded through the hook and sent as a part", async () => {
		backend.inboxState.uploads.length = 0;
		const fake = room("at1", [entry("message", "anna", { text: "hi" }, 0)]);
		const panel = await mountPanel("at1");
		await pasteFile(panel, "note.txt");
		expect(panel.text()).toContain("note.txt");
		await typeAndSubmit(panel, "with file");
		expect(backend.inboxState.uploads).toEqual([
			{ conversationId: "at1", name: "note.txt" },
		]);
		const body = fake.sent.find((item) => item.kind === "message")?.envelope
			?.body as { parts?: { name: string; handle: { app_id: string } }[] };
		expect(body.parts?.[0]).toMatchObject({
			name: "note.txt",
			handle: { app_id: "conversation_at1" },
		});
		await panel.unmount();
	});

	test("a file sends without text", async () => {
		backend.inboxState.uploads.length = 0;
		const fake = room("at5", [entry("message", "anna", { text: "hi" }, 0)]);
		const panel = await mountPanel("at5");
		await pasteFile(panel, "photo.png");
		const send = panel.host.querySelector(
			'form button[type="submit"]',
		) as HTMLButtonElement;
		expect(send.disabled).toBe(false);
		await typeAndSubmit(panel, "");
		expect(backend.inboxState.uploads).toEqual([
			{ conversationId: "at5", name: "photo.png" },
		]);
		const body = fake.sent.find((item) => item.kind === "message")?.envelope
			?.body as { text: string; parts?: { name: string }[] };
		expect(body.text).toBe("");
		expect(body.parts?.[0]).toMatchObject({ name: "photo.png" });
		await panel.unmount();
	});

	test("a failed upload sends nothing and keeps the message to retry", async () => {
		backend.inboxState.failUploads = true;
		const fake = room("at2", [entry("message", "anna", { text: "hi" }, 0)]);
		const panel = await mountPanel("at2");
		await pasteFile(panel, "note.txt");
		await typeAndSubmit(panel, "see file");
		backend.inboxState.failUploads = false;
		expect(fake.sent.filter((item) => item.kind === "message")).toHaveLength(0);
		expect(panel.text()).toContain("Not sent");
		expect(panel.text()).toContain("Retry");
		await panel.unmount();
	});

	test("attachment parts render as files in the message", async () => {
		room("at3", [
			entry(
				"message",
				"bot_scout",
				{
					text: "Here you go",
					parts: [
						{
							kind: "attachment",
							id: "a1",
							name: "report.txt",
							handle: {
								app_id: "conversation_at3",
								path: "f1.txt",
								bytes: 5,
								sha256: "00",
								mime: "text/plain",
							},
						},
					],
				},
				0,
			),
		]);
		const panel = await mountPanel("at3");
		await settle(100);
		expect(panel.text()).toContain("report.txt");
		await panel.unmount();
	});

	test("an encrypted room takes no files", async () => {
		room("at4", [entry("message", "anna", { text: "hi" }, 0)], {
			encryption: "e2e",
		});
		const panel = await mountPanel("at4");
		await pasteFile(panel, "note.txt");
		expect(panel.text()).not.toContain("note.txt");
		await panel.unmount();
	});
});

describe("message bodies", () => {
	const markdown = [
		"**Bold claim** with a [link](https://example.com)",
		"",
		"- first item",
		"- second item",
		"",
		"`inline code` and",
		"",
		"```ts",
		"const answer = 42;",
		"```",
		"",
		"```a2ui",
		'{"id":"root","type":"text","text":"Hello"}',
		"```",
	].join("\n");

	test("render through the chat markdown renderer, not as raw text", async () => {
		room("md", [entry("message", "bot_scout", { text: markdown }, 0)]);
		const panel = await mountPanel("md");
		const host = panel.host;
		expect(host.querySelector("strong")?.textContent).toBe("Bold claim");
		expect(
			host.querySelector("a[data-slate-url='https://example.com']"),
		).not.toBeNull();
		expect(host.querySelectorAll("li").length).toBe(2);
		expect(host.querySelector("code")?.textContent).toContain("inline code");
		expect(host.textContent).not.toContain("**Bold claim**");
		expect(host.textContent).not.toContain("```");
		expect(
			host.querySelector(
				"[data-fl-a2ui-fence], [data-slot='a2ui-fence-skeleton']",
			),
		).not.toBeNull();
		await panel.unmount();
	});

	test("a mention of a member is a coloured, focusable name that opens the profile card; other @text stays plain", async () => {
		room("mn", [
			entry(
				"message",
				"anna",
				{
					text: "Hey @Scout, check this with @nobody",
					mentions: ["bot_scout"],
				},
				0,
			),
		]);
		const panel = await mountPanel("mn");
		const mention = panel.host.querySelector("[data-slate-type='mention']");
		expect(mention?.textContent).toContain("@Scout");
		expect(mention?.className).toContain("text-primary");
		expect(mention?.getAttribute("data-mention-subject")).toBe("bot_scout");
		expect(panel.host.querySelectorAll("[data-mention-subject]")).toHaveLength(
			1,
		);
		expect(panel.text()).toContain("@nobody");
		expect(panel.host.querySelector("li")).toBeNull();
		const trigger = mention?.querySelector("button") as unknown as HTMLElement;
		expect(trigger).not.toBeNull();
		await act(async () => {
			trigger.dispatchEvent(
				new window.FocusEvent("focusin", { bubbles: true }) as never,
			);
		});
		await settle(450);
		const card = window.document.querySelector(
			"[data-slot='hover-card-content']",
		);
		expect(card?.textContent).toContain("Scout");
		await panel.unmount();
	});

	test("bot replies without mention metadata highlight the addressed reader's full name", async () => {
		room(
			"mn-self",
			[
				entry(
					"message",
					"bot_scout",
					{
						text: "@The G: Felix findet die Abfrage alle 5 Minuten zu häufig. (**@Anna Weber**), welche Option soll ich umsetzen? @nobody",
					},
					0,
				),
			],
			{
				members: [
					...conversation.members,
					{
						principal_id: "the-g",
						kind: "user",
						added_by: "me",
						joined_at: "",
					},
				],
			},
		);
		const panel = await mountPanel("mn-self", "the-g");
		const self = panel.host.querySelector('[data-mention-subject="the-g"]');
		expect(self?.querySelector("button")?.textContent).toBe("@The G");
		expect(self?.getAttribute("data-mention-self")).toBe("true");
		expect(self?.className).toContain("text-foreground");
		expect(self?.className).toContain("ring-2");
		expect(self?.querySelector("button")?.getAttribute("aria-label")).toBe(
			"@The G (You)",
		);
		expect(panel.host.querySelectorAll("[data-mention-subject]")).toHaveLength(
			2,
		);
		expect(
			panel.host
				.querySelector('[data-mention-subject="anna"]')
				?.hasAttribute("data-mention-self"),
		).toBe(false);
		expect(panel.text()).toContain("@nobody");
		await panel.unmount();

		const otherReader = await mountPanel("mn-self", "anna");
		expect(
			otherReader.host.querySelector("[data-mention-self] button")?.textContent,
		).toBe("@Anna Weber");
		await otherReader.unmount();
	});

	test("member names in code and external links stay literal", async () => {
		room("mn-literal", [
			entry(
				"message",
				"bot_scout",
				{
					text: "`@Felix Schultz` and [@Anna Weber](https://example.com)\n\n```text\n@Scout\n```",
				},
				0,
			),
		]);
		const panel = await mountPanel("mn-literal");
		expect(panel.host.querySelectorAll("[data-mention-subject]")).toHaveLength(
			0,
		);
		expect(panel.host.querySelector("code")?.textContent).toBe(
			"@Felix Schultz",
		);
		expect(
			panel.host.querySelector('a[data-slate-url="https://example.com"]')
				?.textContent,
		).toBe("@Anna Weber");
		await panel.unmount();
	});

	test("replying sends reply_to and the answer quotes the question", async () => {
		const fake = room("rp", [
			entry("message", "bot_scout", { text: "The question" }, 0, "q1"),
			entry("message", "anna", { text: "The answer", reply_to: "q1" }, 1, "a1"),
		]);
		const panel = await mountPanel("rp");
		expect(panel.text().split("The question").length - 1).toBe(2);
		await click(
			panel.host.querySelector("button[title='Reply to message']") ?? undefined,
		);
		await typeAndSubmit(panel, "Follow up");
		const sent = fake.sent.find((item) => item.kind === "message");
		expect(sent?.envelope?.body).toMatchObject({
			text: "Follow up",
			reply_to: expect.any(String),
		});
		await panel.unmount();
	});

	test("a failed bot turn shows the chat's error card", async () => {
		room("er", [
			entry(
				"message",
				"bot_scout",
				{
					text: "The language model didn't answer.",
					error: { code: "BOT_TURN_MODEL_ERROR" },
				},
				0,
			),
		]);
		const panel = await mountPanel("er");
		expect(panel.host.querySelector("[data-fl-chat-error]")).not.toBeNull();
		expect(panel.text()).toContain("The language model didn't answer.");
		await panel.unmount();
	});

	test("a bot without a language model offers the model picker to its owner only", async () => {
		const reason =
			"I can't answer yet: This profile has no language model yet.";
		room("nm", [
			entry(
				"message",
				"bot_scout",
				{ text: reason, error: { code: "BOT_TURN_NO_MODEL" } },
				0,
			),
		]);
		const picker = "[data-fl-chat-error-model-setup]";

		const visitor = await mountPanel("nm");
		expect(visitor.text()).toContain(reason);
		expect(visitor.host.querySelector(picker)).toBeNull();
		await visitor.unmount();

		const owner = await mountPanel("nm", "anna");
		expect(owner.text()).toContain(reason);
		expect(owner.host.querySelector(picker)).not.toBeNull();
		expect(owner.text()).toContain("Choose a model");
		await owner.unmount();
	});

	test("one speaker line per run; own messages carry no speaker line", async () => {
		room("gp", [
			entry("message", "bot_scout", { text: "one" }, 0),
			entry("message", "bot_scout", { text: "two" }, 1),
			entry("message", "me", { text: "mine" }, 2),
			entry("message", "anna", { text: "three" }, 3),
		]);
		const panel = await mountPanel("gp");
		expect(panel.text().split("Scout").length - 1).toBe(1);
		expect(panel.text().split("Anna Weber").length - 1).toBe(1);
		expect(panel.text()).not.toContain("Felix Schultz");
		expect(
			panel.host.querySelectorAll("[data-fl-chat-message='user']").length,
		).toBe(1);
		await panel.unmount();
	});

	test("a mail the bot sent says so above its text; a chat message doesn't", async () => {
		room("ml", [
			entry(
				"message",
				"bot_scout",
				{
					text: "Hallo Felix",
					mail: { direction: "out", to: ["felix@example.com"], cc: [] },
				},
				0,
			),
			entry("message", "bot_scout", { text: "Done" }, 1),
		]);
		const panel = await mountPanel("ml");
		expect(panel.text()).toContain("Sent by email to felix@example.com");
		expect(panel.text().split("Sent by email").length - 1).toBe(1);
		await panel.unmount();
	});

	test("reactions still toggle on a rendered row", async () => {
		const before = c1.sent.length;
		await click(
			c1Panel.host.querySelector(
				"button[aria-pressed='true'][aria-label^='👍']",
			) ?? undefined,
		);
		const reaction = c1.sent
			.slice(before)
			.find((item) => item.kind === "reaction");
		expect(reaction?.envelope?.body).toMatchObject({
			target: "s2",
			emoji: "👍",
			op: "remove",
		});
	});

	test("no stale footer about the bot answering elsewhere", async () => {
		const dm = room("dm", [entry("message", "bot_scout", { text: "Hi" }, 0)], {
			kind: "direct",
			members: [
				{ principal_id: "me", kind: "user", added_by: null, joined_at: "" },
				{
					principal_id: "bot_scout",
					kind: "bot",
					added_by: "me",
					joined_at: "",
				},
			],
		});
		expect(dm.conversation.kind).toBe("direct");
		const panel = await mountPanel("dm");
		expect(panel.text()).not.toContain("answers in its own chat");
		await panel.unmount();
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
		const before = fake.queries.length;
		await click(button(panel, "Load older messages"));
		expect(fake.queries.slice(before).some((query) => query.before)).toBe(true);
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

describe("deletion on another client", () => {
	test("a canceled detail request's delayed 404 cannot clear a rejoined conversation", async () => {
		const fake = room("remote-canceled-detail", [
			entry("message", "anna", { text: "Rejoined conversation history" }, 0),
		]);
		let closed = 0;
		const panel = await mountPanel(fake.conversation.id, "me", () => {
			closed += 1;
		});
		let rejectOld: (error: Error) => void = () => {};
		try {
			fake.beforeGet = () =>
				new Promise<void>((_, reject) => {
					rejectOld = reject;
				});
			let staleRead: Promise<void> | undefined;
			await act(async () => {
				staleRead = client.refetchQueries({
					queryKey: inboxKeys.conversation(fake.conversation.id),
					exact: true,
				});
			});
			fake.beforeGet = undefined;
			await act(async () => {
				await client.cancelQueries({
					queryKey: inboxKeys.conversation(fake.conversation.id),
					exact: true,
				});
				await client.refetchQueries({
					queryKey: inboxKeys.conversation(fake.conversation.id),
					exact: true,
				});
			});
			await act(async () => {
				rejectOld(
					new ApiResponseError({
						status: 404,
						code: "NOT_FOUND",
						message: "Old missing membership",
					}),
				);
				await staleRead;
			});
			await settle();
			expect(panel.text()).toContain("Rejoined conversation history");
			expect(
				client.getQueryData<unknown>(inboxKeys.entries(fake.conversation.id)),
			).toBeDefined();
			expect(closed).toBe(0);
		} finally {
			await panel.unmount();
		}
	});

	test("rechecks a previously missing room before closing it after a rejoin", async () => {
		const fake = room("remote-rejoin", [
			entry("message", "anna", { text: "After rejoining" }, 0),
		]);
		client.setQueryData(inboxKeys.conversation(fake.conversation.id), null);
		let closed = 0;
		const panel = await mountPanel(fake.conversation.id, "me", () => {
			closed += 1;
		});
		try {
			expect(panel.text()).toContain("After rejoining");
			expect(closed).toBe(0);
		} finally {
			await panel.unmount();
		}
	});

	test("a refreshed list cannot seed a new boundary over old cached history", async () => {
		const fake = room("remote-list", [], {
			kind: "direct",
			history_cleared_at: new Date(BASE + 1000).toISOString(),
		});
		client.setQueryData(inboxKeys.conversations(), [fake.conversation]);
		client.setQueryData(inboxKeys.entries(fake.conversation.id), {
			pages: [
				{
					entries: [
						entry("message", "anna", { text: "Old list-seeded history" }, 0),
					],
					next_before: null,
				},
			],
			pageParams: [undefined],
		});
		const panel = await mountPanel(fake.conversation.id);
		try {
			expect(panel.text()).not.toContain("Old list-seeded history");
		} finally {
			await panel.unmount();
		}
	});

	test("a changed history boundary clears the open chat and detaches its old feed", async () => {
		const fake = room(
			"remote-clear",
			[entry("message", "anna", { text: "History to clear" }, 0)],
			{ kind: "direct" },
		);
		let oldFeed: ConversationFeedListener | undefined;
		registerConversationFeed({
			watch(target, listener) {
				if (target.conversationId === fake.conversation.id && !oldFeed)
					oldFeed = listener;
				return () => {};
			},
		});
		const panel = await mountPanel(fake.conversation.id);
		try {
			expect(panel.text()).toContain("History to clear");
			client.setQueryData(
				inboxKeys.entries(fake.conversation.id, "bot_scout"),
				{ copy: "bot history" },
			);
			fake.entries = [];
			fake.conversation = {
				...fake.conversation,
				history_cleared_at: new Date(BASE + 1000).toISOString(),
			};
			await act(async () => {
				await client.invalidateQueries({
					queryKey: inboxKeys.conversation(fake.conversation.id),
				});
			});
			await settle(100);
			expect(panel.text()).not.toContain("History to clear");
			expect(
				client.getQueryData<unknown>(
					inboxKeys.entries(fake.conversation.id, "bot_scout"),
				),
			).toEqual({ copy: "bot history" });
			await act(async () =>
				oldFeed?.([entry("message", "anna", { text: "Delayed old feed" }, 10)]),
			);
			await settle();
			expect(panel.text()).not.toContain("Delayed old feed");
			fake.entries.push(
				entry("message", "anna", { text: "After deletion" }, 20),
			);
			await act(async () => {
				await client.invalidateQueries({
					queryKey: inboxKeys.entries(fake.conversation.id),
					exact: true,
				});
			});
			await settle();
			expect(panel.text()).toContain("After deletion");
		} finally {
			await panel.unmount();
			registerConversationFeed(null);
		}
	});

	test.each([
		new ApiResponseError({
			status: 404,
			code: "NOT_FOUND",
			message: "Conversation not found",
		}),
		Object.assign(new Error("Local conversation not found"), {
			name: "LocalStoreError",
			code: "NOT_FOUND",
		}),
	])(
		"a confirmed missing group clears cached history and closes the panel",
		async (error) => {
			const fake = room(`remote-left-${error.name}`, [
				entry("message", "anna", { text: "Former room history" }, 0),
			]);
			let closed = 0;
			const panel = await mountPanel(fake.conversation.id, "me", () => {
				closed += 1;
			});
			try {
				fake.getError = error;
				await act(async () => {
					await client.invalidateQueries({
						queryKey: inboxKeys.conversation(fake.conversation.id),
					});
				});
				await settle();
				expect(panel.text()).not.toContain("Former room history");
				expect(
					client.getQueryData<unknown>(inboxKeys.entries(fake.conversation.id)),
				).toBeUndefined();
				expect(closed).toBe(1);
			} finally {
				await panel.unmount();
			}
		},
	);

	test.each([
		new Error("offline"),
		new ApiResponseError({ status: 404, message: "Proxy route missing" }),
	])(
		"keeps the open history on a temporary or uncoded error",
		async (error) => {
			const fake = room(`remote-error-${error.name}`, [
				entry("message", "anna", { text: "Keep this history" }, 0),
			]);
			let closed = 0;
			const panel = await mountPanel(fake.conversation.id, "me", () => {
				closed += 1;
			});
			try {
				fake.getError = error;
				await act(async () => {
					await client.invalidateQueries({
						queryKey: inboxKeys.conversation(fake.conversation.id),
					});
				});
				await settle();
				expect(panel.text()).toContain("Keep this history");
				expect(
					client.getQueryData<unknown>(inboxKeys.entries(fake.conversation.id)),
				).toBeDefined();
				expect(closed).toBe(0);
			} finally {
				await panel.unmount();
			}
		},
	);
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

	test("only your own messages offer the chat's edit action", async () => {
		room("edit", [
			entry("message", "me", { text: "Meet at 5" }, 0, "mine"),
			entry("message", "anna", { text: "Okay" }, 1, "theirs"),
		]);
		const panel = await mountPanel("edit");
		const editButtons = () =>
			panel.host.querySelectorAll("button[title='Edit message']");
		expect(editButtons()).toHaveLength(1);
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

describe("room settings", () => {
	async function setInput(input: Element | null, text: string) {
		const setValue = Object.getOwnPropertyDescriptor(
			window.HTMLInputElement.prototype,
			"value",
		)?.set;
		await act(async () => {
			setValue?.call(input, text);
			input?.dispatchEvent(
				new window.Event("input", { bubbles: true }) as never,
			);
		});
	}

	test("renaming a group from its menu saves the name and shows it in the header", async () => {
		room("renamable", newestFirst);
		const panel = await mountPanel("renamable");
		await chooseMenuItem(
			button(panel, "Conversation options"),
			"Edit name and picture",
		);
		const dialog = window.document.querySelector("[role='dialog']");
		expect(dialog?.textContent).toContain("Edit room");
		await setInput(
			(dialog?.querySelector("input:not([type='file'])") ??
				null) as unknown as Element | null,
			"  Launch crew ",
		);
		const form = dialog?.querySelector("form") as unknown as HTMLFormElement;
		await act(async () => {
			form.dispatchEvent(
				new window.Event("submit", {
					bubbles: true,
					cancelable: true,
				}) as never,
			);
		});
		await settle(50);
		expect(backend.inboxState.renamed).toContainEqual({
			conversationId: "renamable",
			title: "Launch crew",
		});
		expect(window.document.querySelector("[role='dialog']")).toBeNull();
		expect(panel.text()).toContain("Launch crew");
		await panel.unmount();
	});

	test("a group's picture replaces the member stack in the header", async () => {
		room("pictured", newestFirst, {
			image_url: "https://files.test/room.png",
		});
		const panel = await mountPanel("pictured");
		const picture = panel.host.querySelector(
			"img[src='https://files.test/room.png']",
		);
		expect(picture).not.toBeNull();
		await panel.unmount();
	});

	test("renames and picture changes show as pills in the history", async () => {
		room("changed", [
			entry(
				"room",
				"me",
				{ change: "image", removed: true, by: "me" },
				21,
				"r2",
			),
			entry(
				"room",
				"anna",
				{ change: "title", title: "Launch crew", by: "anna" },
				20,
				"r1",
			),
			...newestFirst,
		]);
		const panel = await mountPanel("changed");
		const pills = [
			...panel.host.querySelectorAll("[data-slot='room-change']"),
		].map((pill) => pill.textContent);
		expect(pills).toEqual([
			"Anna Weber renamed the room to “Launch crew”",
			"You removed the room's picture",
		]);
		await panel.unmount();
	});
});

describe("blocking in an existing conversation", () => {
	async function confirm(label: string) {
		const dialog = window.document.querySelector("[role='alertdialog']");
		const action = [...(dialog?.querySelectorAll("button") ?? [])].find(
			(button) => button.textContent === label,
		);
		await click(action as unknown as Element);
		await settle(50);
	}

	test("blocking disables retry of a failed send while still allowing discard", async () => {
		backend.inboxState.blocks = [];
		await act(async () => client.setQueryData(["inbox", "blocks"], []));
		const fake = room("block-retry", [], {
			kind: "direct",
			members: conversation.members.filter((member) => member.kind === "user"),
		});
		fake.beforeSend = async () => {
			throw new Error("offline");
		};
		const panel = await mountPanel("block-retry");
		await typeAndSubmit(panel, "Unsent message");
		expect((button(panel, "Retry") as HTMLButtonElement).disabled).toBe(false);
		await chooseMenuItem(button(panel, "Conversation options"), "Block user");
		await confirm("Block");
		expect((button(panel, "Retry") as HTMLButtonElement).disabled).toBe(true);
		await click(button(panel, "Discard"));
		expect(button(panel, "Retry")).toBeUndefined();
		expect(fake.sent).toHaveLength(1);
		await panel.unmount();
	});

	test("blocks a person from the menu, keeps history readable, and enables writing after unblock", async () => {
		backend.inboxState.blocks = [];
		client.setQueryData(["inbox", "blocks"], []);
		room(
			"block-person",
			[entry("message", "anna", { text: "Existing history" }, 0)],
			{
				kind: "direct",
				members: conversation.members.filter(
					(member) => member.kind === "user",
				),
			},
		);
		const panel = await mountPanel("block-person");
		await chooseMenuItem(button(panel, "Conversation options"), "Block user");
		await confirm("Block");
		expect(backend.inboxState.blocks[0]?.principal_id).toBe("anna");
		expect(panel.text()).toContain("Existing history");
		const composer = panel.host.querySelector(
			"textarea",
		) as HTMLTextAreaElement;
		expect(
			(panel.host.querySelector('button[type="submit"]') as HTMLButtonElement)
				.disabled,
		).toBe(true);
		expect(composer.placeholder).toContain("This conversation is blocked");
		await chooseMenuItem(button(panel, "Conversation options"), "Unblock");
		await confirm("Unblock");
		expect(backend.inboxState.blocks).toEqual([]);
		await setText(panel.host.querySelector("textarea"), "Allowed again");
		expect(
			(panel.host.querySelector('button[type="submit"]') as HTMLButtonElement)
				.disabled,
		).toBe(false);
		await panel.unmount();
	});

	test("a person's block disables their bot chat while shared rooms remain writable", async () => {
		backend.inboxState.blocks = [
			{ principal_id: "anna", kind: "user", created_at: "" },
		];
		await act(async () =>
			client.setQueryData(["inbox", "blocks"], backend.inboxState.blocks),
		);
		room(
			"blocked-bot",
			[entry("message", "bot_scout", { text: "Bot history" }, 0)],
			{
				kind: "direct",
				members: conversation.members.filter(
					(member) => member.principal_id !== "anna",
				),
			},
		);
		const panel = await mountPanel("blocked-bot");
		expect(
			(panel.host.querySelector("textarea") as HTMLTextAreaElement).placeholder,
		).toContain("This conversation is blocked");
		await typeAndSubmit(panel, "Must not send");
		expect(roomOf("blocked-bot").sent).toEqual([]);
		expect(panel.text()).toContain("Bot history");
		expect(
			(c1Panel.host.querySelector("textarea") as HTMLTextAreaElement)
				.placeholder,
		).not.toContain("This conversation is blocked");
		await panel.unmount();
		backend.inboxState.blocks = [];
		await act(async () => client.setQueryData(["inbox", "blocks"], []));
	});
});
