import {
	afterAll,
	afterEach,
	beforeAll,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement, useState } from "react";
import type {
	ConversationKind,
	IConversation,
} from "../../state/backend-state/inbox-state";
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
const removals: string[] = [];
let remove: () => Promise<void> = async () => {};
const actualBackend = { ...(await import("../../state/backend-state")) };
mock.module("../../state/backend-state", () => ({
	...actualBackend,
	useBackend: () => ({
		inboxState: {
			deleteConversation: async (id: string) => {
				removals.push(id);
				await remove();
			},
		},
	}),
}));

const { createRoot } = await import("react-dom/client");
const { inboxKeys } = await import("../../hooks/use-inbox");
const { inboxMarkKeys } = await import("../../hooks/use-inbox-marks");
const { DeleteConversationDialog } = await import(
	"./delete-conversation-dialog"
);
const { ConversationMenu } = await import("./conversation-menu");
let container: HTMLElement;
let root: ReturnType<typeof createRoot>;
let client: QueryClient;
let closed = 0;

beforeAll(installDomGlobals);
afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	client.clear();
});
afterAll(() => {
	mock.module("@flow-like/locales", () => actualLocales);
	mock.module("../../state/backend-state", () => actualBackend);
	restoreDomGlobals();
});

function Harness({ kind, menu }: { kind: ConversationKind; menu: boolean }) {
	const [open, setOpen] = useState(true);
	const close = () => {
		closed += 1;
	};
	if (menu) {
		return createElement(ConversationMenu, {
			conversation: {
				id: "room-1",
				kind,
				title: null,
				members: [{ kind: "user", principal_id: "me" }],
			} as IConversation,
			me: "me",
			onLeave: close,
		});
	}
	return createElement(DeleteConversationDialog, {
		open,
		onOpenChange: setOpen,
		conversationId: "room-1",
		kind,
		onDelete: close,
	});
}

async function show({
	handler = async () => {},
	kind = "direct",
	menu = false,
}: {
	handler?: () => Promise<void>;
	kind?: ConversationKind;
	menu?: boolean;
} = {}) {
	removals.length = 0;
	closed = 0;
	remove = handler;
	client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
	client.setQueryData(inboxKeys.conversations(), [
		{ id: "room-1" },
		{ id: "room-2" },
	]);
	client.setQueryData(inboxKeys.conversation("room-1"), { id: "room-1" });
	client.setQueryData(inboxKeys.entries("room-1"), {
		pages: [{ entries: [{ id: "old" }] }],
		pageParams: [undefined],
	});
	client.setQueryData(inboxMarkKeys.pins("room-1"), [{ message_id: "old" }]);
	client.setQueryData(inboxMarkKeys.stars("room-1"), {
		stars: [{ message_id: "old" }],
	});
	client.setQueryData(inboxMarkKeys.stars(), {
		pages: [
			{
				stars: [
					{ message_id: "old", conversation_id: "room-1" },
					{ message_id: "keep", conversation_id: "room-2" },
				],
				next_before: null,
			},
		],
		pageParams: [undefined],
	});
	client.setQueryData(["inbox", "file", "room-1", "file-1", "me"], {
		url: "old-person-url",
	});
	client.setQueryData(["inbox", "file", "room-1", "file-1", "bot-1"], {
		url: "bot-url",
	});
	client.setQueryData(inboxKeys.conversations("bot-1"), [{ id: "room-1" }]);
	client.setQueryData(inboxKeys.entries("room-1", "bot-1"), { copy: "bot" });
	client.setQueryData(inboxMarkKeys.pins("room-1", "bot-1"), [
		{ message_id: "old" },
	]);
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	await act(async () =>
		root.render(
			createElement(
				QueryClientProvider,
				{ client },
				createElement(Harness, { kind, menu }),
			),
		),
	);
}

function button(label: string) {
	const found = [...window.document.querySelectorAll("button")].find(
		(item) =>
			item.textContent === label || item.getAttribute("aria-label") === label,
	);
	if (!found) throw new Error(`Missing button: ${label}`);
	return found;
}

describe("deleting a conversation", () => {
	test("explains personal deletion and how new messages return, without deleting on cancel", async () => {
		await show();
		expect(window.document.body.textContent).toContain(
			"Other participants keep their copies.",
		);
		expect(window.document.body.textContent).toContain(
			"your old history still cleared",
		);
		await act(async () => button("Cancel").click());
		expect(removals).toEqual([]);
		expect(closed).toBe(0);
		expect(
			client.getQueryData<unknown>(inboxKeys.entries("room-1")),
		).toBeDefined();
	});

	test("deletes the user's view and cached history while preserving other conversations and bot copies", async () => {
		await show();
		await act(async () => button("Delete conversation").click());
		await settle();
		expect(removals).toEqual(["room-1"]);
		expect(closed).toBe(1);
		expect(client.getQueryData<unknown>(inboxKeys.conversations())).toEqual([
			{ id: "room-2" },
		]);
		for (const key of [
			inboxKeys.conversation("room-1"),
			inboxKeys.entries("room-1"),
			inboxMarkKeys.pins("room-1"),
			inboxMarkKeys.stars("room-1"),
			["inbox", "file", "room-1", "file-1", "me"],
		]) {
			expect(client.getQueryData<unknown>(key)).toBeUndefined();
		}
		expect(client.getQueryData<unknown>(inboxMarkKeys.stars())).toEqual({
			pages: [
				{
					stars: [{ message_id: "keep", conversation_id: "room-2" }],
					next_before: null,
				},
			],
			pageParams: [undefined],
		});
		expect(client.getQueryState(inboxKeys.conversations())?.isInvalidated).toBe(
			true,
		);
		expect(client.getQueryState(inboxMarkKeys.stars())?.isInvalidated).toBe(
			true,
		);
		expect(
			client.getQueryData<unknown>(inboxKeys.entries("room-1", "bot-1")),
		).toEqual({
			copy: "bot",
		});
		expect(
			client.getQueryData<unknown>(inboxKeys.conversations("bot-1")),
		).toEqual([{ id: "room-1" }]);
		expect(
			client.getQueryData<unknown>(inboxMarkKeys.pins("room-1", "bot-1")),
		).toEqual([{ message_id: "old" }]);
		expect(
			client.getQueryData<unknown>([
				"inbox",
				"file",
				"room-1",
				"file-1",
				"bot-1",
			]),
		).toEqual({ url: "bot-url" });
		expect(window.document.querySelector('[role="alertdialog"]')).toBeNull();
	});

	test("cancels an older history read so it cannot restore deleted cached messages", async () => {
		await show();
		let finish: (value: unknown) => void = () => {};
		const read = client
			.fetchQuery({
				queryKey: inboxKeys.entries("room-1"),
				queryFn: () =>
					new Promise((resolve) => {
						finish = resolve;
					}),
			})
			.catch(() => undefined);
		await act(async () => button("Delete conversation").click());
		await settle();
		finish({ old: "messages" });
		await read;
		expect(
			client.getQueryData<unknown>(inboxKeys.entries("room-1")),
		).toBeUndefined();
	});

	test("group confirmation explains that the owner and bots leave and others keep their copies", async () => {
		await show({ kind: "group" });
		expect(window.document.body.textContent).toContain(
			"You and your bots will leave this room",
		);
		expect(window.document.body.textContent).toContain(
			"Other participants keep their copies.",
		);
		await act(async () => button("Leave and delete").click());
		await settle();
		expect(removals).toEqual(["room-1"]);
		expect(closed).toBe(1);
	});

	test("keeps history and the confirmation on failure, then permits retry", async () => {
		await show({
			handler: async () => {
				throw new Error("offline");
			},
		});
		await act(async () => button("Delete conversation").click());
		await settle();
		expect(closed).toBe(0);
		expect(window.document.querySelector('[role="alert"]')).not.toBeNull();
		expect(
			client.getQueryData<unknown>(inboxKeys.entries("room-1")),
		).toBeDefined();
		remove = async () => {};
		await act(async () => button("Delete conversation").click());
		await settle();
		expect(removals).toHaveLength(2);
		expect(closed).toBe(1);
	});

	test("disables confirmation and cancellation while deleting", async () => {
		let finish = () => {};
		await show({
			handler: () =>
				new Promise<void>((resolve) => {
					finish = resolve;
				}),
		});
		await act(async () => button("Delete conversation").click());
		await settle();
		expect(button("Deleting…").disabled).toBe(true);
		expect(button("Cancel").disabled).toBe(true);
		expect(closed).toBe(0);
		await act(async () => finish());
		await settle();
		expect(closed).toBe(1);
	});

	test.each(["direct", "group"] as const)(
		"offers deletion in the %s conversation menu",
		async (kind) => {
			await show({ kind, menu: true });
			await act(async () =>
				button("Conversation options").dispatchEvent(
					new window.PointerEvent("pointerdown", {
						bubbles: true,
						button: 0,
						ctrlKey: false,
					}),
				),
			);
			await settle();
			const action =
				kind === "group" ? "Leave and delete" : "Delete conversation";
			const item = [
				...window.document.querySelectorAll('[role="menuitem"]'),
			].find((item) => item.textContent === action);
			expect(item).toBeDefined();
			if (kind === "group")
				expect(window.document.body.textContent).toContain("Leave room");
			await act(async () => (item as unknown as HTMLElement).click());
			await settle();
			expect(
				window.document.querySelector('[role="alertdialog"]'),
			).not.toBeNull();
			expect(removals).toEqual([]);
		},
	);
});
