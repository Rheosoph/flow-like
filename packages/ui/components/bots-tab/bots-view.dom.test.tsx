import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { Fragment, act, createElement } from "react";
import type { IBot } from "../../state/backend-state/bot-state";
import type { IConversation } from "../../state/backend-state/inbox-state";
import type { IUserLookup } from "../../state/backend-state/types";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	settle,
	translate,
	domWindow as window,
} from "./testing/dom-harness";

const navigation = {
	params: new URLSearchParams(),
	replace: mock((href: string) => {
		navigation.params = new URLSearchParams(href.split("?")[1] ?? "");
		window.history.replaceState(null, "", href);
	}),
};
// Like the real restore, the conversation (and with it its bot) comes on screen and is remembered.
const restoreConversation = mock(async (conversationId: string) => {
	useGlobalChatStore.getState().loadConversation(conversationId, []);
	rememberGlobalChatConversation(conversationId);
	return true;
});

const CHAT_BODY = "../global-chat/global-chat-body";
const CHAT_STREAM = "../../state/global-chat/global-chat-stream";
const EDITOR = "./editor";
const BOT_EXPRESSION = "../../hooks/use-bot-expression";
const renderEditor = mock(
	(_props: {
		open: boolean;
		botId?: string | null;
		onOpenChange: (open: boolean) => void;
		onDuplicated?: (bot: IBot) => void;
	}) => null,
);

// bun keeps module mocks for every later file in the process, so each one spreads the real
// module and is put back in afterAll.
const actual = {
	locales: { ...(await import("@flow-like/locales")) },
	navigation: { ...(await import("next/navigation")) },
	oidc: { ...(await import("react-oidc-context")) },
	chatBody: { ...(await import(CHAT_BODY)) },
	chatStream: { ...(await import(CHAT_STREAM)) },
	editor: { ...(await import(EDITOR)) },
	expression: { ...(await import(BOT_EXPRESSION)) },
};
// The emotion model warms up after an avatar mounts and would update it outside act.
mock.module(BOT_EXPRESSION, () => ({
	...actual.expression,
	useBotExpression: () => ({ expression: "neutral", status: "idle" }),
}));
mock.module("@flow-like/locales", () => ({
	...actual.locales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));
mock.module("next/navigation", () => ({
	...actual.navigation,
	useRouter: () => ({ replace: navigation.replace, push: () => {} }),
	usePathname: () => "/bots",
	useSearchParams: () => navigation.params,
}));
mock.module("react-oidc-context", () => ({
	...actual.oidc,
	useAuth: () => ({
		isAuthenticated: true,
		user: {
			access_token: "token",
			profile: { sub: "me", given_name: "Felix" },
		},
	}),
}));
mock.module(CHAT_BODY, () => ({
	...actual.chatBody,
	GlobalChatBody: () =>
		createElement("div", { "data-testid": "global-chat" }, "Global chat"),
}));
mock.module(CHAT_STREAM, () => ({
	...actual.chatStream,
	restoreGlobalChatConversation: restoreConversation,
}));
mock.module(EDITOR, () => ({
	...actual.editor,
	BotEditorDialog: renderEditor,
}));

const { createRoot } = await import("react-dom/client");
const { QueryClient, QueryClientProvider } = await import(
	"@tanstack/react-query"
);
const { useAuthStatusStore, useBackendStore } = await import(
	"../../state/backend-state"
);
const { rememberGlobalChatConversation, useGlobalChatStore } = await import(
	"../../state/global-chat/global-chat-store"
);
const { makeGlobalChatMessage } = await import(
	"../../state/global-chat/global-chat-stream"
);
const { IRole } = await import("../../lib/schema/llm/history");
const { useChatActivityStore } = await import(
	"../../state/chat-activity-store"
);
const { ChatActivityMenu } = await import("../global-chat/chat-activity");
const { botKeys } = await import("../../hooks/use-bots");
const { inboxKeys } = await import("../../hooks/use-inbox");
const { BotsView } = await import("./bots-view");

function bot(id: string, overrides: Partial<IBot> = {}): IBot {
	return {
		id,
		owner_id: "me",
		system: null,
		preset_id: null,
		name: null,
		description: null,
		instructions: null,
		appearance: {} as IBot["appearance"],
		model: { primary: null, fallbacks: [] },
		classification: {} as IBot["classification"],
		capabilities: [],
		profile_id: null,
		location: "server",
		device_id: null,
		created_at: "",
		updated_at: "",
		...overrides,
	};
}

const bots = [
	bot("bot_general", { system: "general" }),
	bot("bot_flux", { system: "flux" }),
	bot("bot_scout", {
		name: "Scout",
		description: "Research digest, weekdays at 7:00",
		capabilities: ["internet", "encryption"],
	}),
];

const now = new Date().toISOString();

function member(
	principal_id: string,
	kind: "user" | "bot" = "user",
): IConversation["members"][number] {
	return { principal_id, kind, added_by: null, joined_at: now };
}

const conversations: IConversation[] = [
	{
		id: "c1",
		kind: "group",
		title: "Q4 launch",
		encryption: "plain",
		created_by: "me",
		created_at: now,
		last_activity_at: now,
		members: [member("me"), member("anna"), member("bot_scout", "bot")],
		unread_count: 2,
		last_entry: {
			id: "e1",
			message_id: "m1",
			conversation_id: "c1",
			sender_id: "bot_scout",
			kind: "message",
			envelope: { v: 1, enc: "none", body: { text: "Digest is ready." } },
			created_at: now,
			cursor: "1:e1",
		},
	},
	{
		id: "c2",
		kind: "direct",
		title: null,
		encryption: "plain",
		created_by: "anna",
		created_at: now,
		last_activity_at: now,
		members: [member("me"), member("anna")],
		unread_count: 0,
		last_entry: null,
	},
];

const users: Record<string, IUserLookup> = {
	anna: { id: "anna", name: "Anna Weber", created_at: "" },
};

const backend = {
	botState: { listBots: async () => bots },
	appState: { getApps: async () => [] },
	inboxState: {
		listConversations: async () => conversations,
	},
	userState: {
		getInfo: async () => ({ id: "me", name: "Felix Schultz" }),
		getProfile: async () => null,
		lookupUser: async (id: string) => users[id] ?? null,
		lookupUsers: async (ids: string[]) =>
			ids.map((id) => users[id]).filter(Boolean),
	},
};

const previousBackend = useBackendStore.getState().backend;
const previousChat = useGlobalChatStore.getState();
const client = new QueryClient({
	defaultOptions: { queries: { retry: false } },
});
let root: ReturnType<typeof createRoot>;
let container: HTMLElement;

function buttonContaining(...texts: string[]) {
	const matches = (button: HTMLButtonElement) =>
		texts.every((text) => button.textContent?.includes(text));
	return Array.from(container.querySelectorAll("button")).find(matches);
}

function showsGlobalChat() {
	return container.querySelector("aside [data-testid='global-chat']") !== null;
}

async function click(element: { click(): void } | null | undefined) {
	expect(element ?? undefined).toBeDefined();
	await act(async () => {
		element?.click();
	});
	await settle();
}

beforeAll(async () => {
	installDomGlobals();
	useAuthStatusStore.getState().setSignedIn(true);
	useBackendStore.getState().setBackend(backend as never);
	useGlobalChatStore.setState({ mode: "overlay" });
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	const view = createElement(
		Fragment,
		null,
		createElement(ChatActivityMenu),
		createElement(BotsView),
	);
	await act(async () =>
		root.render(createElement(QueryClientProvider, { client }, view)),
	);
	await settle();
	await settle(100);
});

afterAll(async () => {
	await act(async () => root.unmount());
	client.clear();
	useBackendStore.setState({ backend: previousBackend });
	useGlobalChatStore.setState(previousChat, true);
	useChatActivityStore.getState().clear();
	window.sessionStorage.clear();
	window.history.replaceState(null, "", "/bots");
	await new Promise((resolve) => setTimeout(resolve, 0));
	mock.restore();
	mock.module("@flow-like/locales", () => actual.locales);
	mock.module("next/navigation", () => actual.navigation);
	mock.module("react-oidc-context", () => actual.oidc);
	mock.module(CHAT_BODY, () => actual.chatBody);
	mock.module(CHAT_STREAM, () => actual.chatStream);
	mock.module(EDITOR, () => actual.editor);
	mock.module(BOT_EXPRESSION, () => actual.expression);
	restoreDomGlobals();
});

function activeConversation() {
	return useGlobalChatStore.getState().activeConversationId;
}

function dockText() {
	return container.querySelector("aside")?.textContent ?? "";
}

function dockTitle() {
	return container.querySelector(
		"aside [data-slot='bot-pill'] [data-slot='bot-pill-name']",
	)?.textContent;
}

function panelText() {
	return (
		container.querySelector("aside section[aria-label^='About']")
			?.textContent ?? ""
	);
}

async function openPanel() {
	await click(
		container.querySelector<HTMLButtonElement>("aside [data-slot='bot-pill']"),
	);
}

function panelButton(label: string) {
	return Array.from(
		container.querySelectorAll<HTMLButtonElement>(
			"aside section[aria-label^='About'] button",
		),
	).find(
		(button) =>
			button.textContent === label ||
			button.getAttribute("aria-label") === label,
	);
}

describe("BotsView", () => {
	test("closes the floating chat and opens the Assistant's own chat, named by its account id", async () => {
		expect(useGlobalChatStore.getState().mode).toBe("closed");
		expect(showsGlobalChat()).toBe(true);
		expect(dockTitle()).toBe("Assistant");
		expect(activeConversation()).toBe("bot:bot_general");
		await openPanel();
		expect(panelText()).toContain("Your everyday helper");
		await click(panelButton("Back to the chat"));
	});

	test("greets the user and summarizes what needs them", () => {
		const text = container.textContent ?? "";
		expect(text).toContain(", Felix");
		expect(text).toContain("Nothing needs you · 3 bots");
		expect(buttonContaining("Open chat")).toBeUndefined();
	});

	test("lists the crew with built-in, location and encryption badges", () => {
		const text = container.textContent ?? "";
		expect(text).toContain("Your crew");
		expect(text.split("Built in").length - 1).toBe(2);
		expect(text).toContain("Research digest, weekdays at 7:00");
		expect(text).toContain("Cloud");
		expect(
			container.querySelector("[aria-label='End-to-end encrypted']"),
		).not.toBeNull();
		expect(text).not.toContain("Shared with you");
	});

	test("shows rooms and people", () => {
		const text = container.textContent ?? "";
		expect(text).toContain("Q4 launch");
		expect(text).toContain("2 people · 1 bot");
		expect(text).toContain("Scout: Digest is ready.");
		expect(text).toContain("Anna Weber");
		expect(text).toContain("No messages yet");
	});

	test("opens a custom bot's chat with where it runs and what it may do, and mirrors it in the URL", async () => {
		await click(buttonContaining("Scout", "Runs in the cloud"));
		expect(navigation.replace).toHaveBeenLastCalledWith("/bots?bot=bot_scout", {
			scroll: false,
		});
		expect(showsGlobalChat()).toBe(true);
		expect(activeConversation()).toBe("bot:bot_scout");
		expect(dockTitle()).toBe("Scout");
		expect(dockText()).not.toContain("Research digest, weekdays at 7:00");
		await openPanel();
		expect(navigation.replace).toHaveBeenLastCalledWith(
			"/bots?bot=bot_scout&panel=details",
			{ scroll: false },
		);
		const details = panelText();
		expect(details).toContain("Research digest, weekdays at 7:00");
		expect(details).toContain("Cloud");
		expect(details).toContain("Internet");
		expect(details).toContain("End-to-end encryption");
		expect(details).not.toContain("runtime is live");
		expect(details).not.toContain("Move to your account");
		expect(panelButton("Edit")).toBeDefined();
		await click(panelButton("Back to the chat"));
	});

	test("each bot keeps its one conversation, and Flux keeps today's chat", async () => {
		await click(buttonContaining("Assistant", "Built in"));
		expect(activeConversation()).toBe("bot:bot_general");
		await click(buttonContaining("Flux", "Built in"));
		expect(navigation.replace).toHaveBeenLastCalledWith("/bots?bot=flux", {
			scroll: false,
		});
		expect(activeConversation().startsWith("bot:")).toBe(false);
		expect(dockTitle()).toBe("Flux");
		await openPanel();
		expect(panelText()).toContain("Builds and fixes your apps and workflows");
		expect(panelText()).toContain("FlowPilot");
		expect(panelText()).not.toContain("No abilities yet");
		await click(panelButton("Edit"));
		const editor = renderEditor.mock.calls.at(-1)?.[0];
		expect(editor?.open).toBe(true);
		expect(editor?.botId).toBe("bot_flux");
		await act(async () => editor?.onOpenChange(false));
		await click(panelButton("Back to the chat"));
		const fluxConversation = activeConversation();
		await act(async () => {
			useGlobalChatStore
				.getState()
				.appendMessage(
					makeGlobalChatMessage(
						IRole.User,
						"Build me an app",
						fluxConversation,
					),
				);
		});
		await click(buttonContaining("Scout", "Runs in the cloud"));
		expect(activeConversation()).toBe("bot:bot_scout");
		expect(dockTitle()).toBe("Scout");
		await click(buttonContaining("Flux", "Built in"));
		expect(activeConversation()).toBe(fluxConversation);
		expect(dockTitle()).toBe("Flux");
	});

	test("an explicit approval appears in Needs you and opens its conversation", async () => {
		await act(async () => {
			client.setQueryData<IConversation[]>(inboxKeys.conversations(), (list) =>
				list?.map((conversation): IConversation => {
					if (conversation.id !== "c1" || !conversation.last_entry)
						return conversation;
					return {
						...conversation,
						last_entry: {
							...conversation.last_entry,
							envelope: {
								v: 1,
								enc: "none",
								body: {
									text: "Send the digest",
									approval: {
										id: "ap-1",
										turn_id: "turn-1",
										tool: "send_message",
										summary: "Send the digest",
										scope: "",
										args_hash: "h",
									},
								},
							},
						},
					};
				}),
			);
		});
		await settle();
		expect(container.textContent).toContain("1 thing needs you · 3 bots");
		await click(buttonContaining("Open chat"));
		expect(navigation.replace).toHaveBeenLastCalledWith(
			"/bots?conversation=c1",
			{ scroll: false },
		);
		expect(buttonContaining("Open chat")).toBeUndefined();
	});

	test("keeps the room open when the global chat switches conversations on its own", async () => {
		await act(async () => {
			useGlobalChatStore.setState({ activeConversationId: "chat-2" });
		});
		await settle();
		expect(container.querySelector("aside")).not.toBeNull();
		expect(showsGlobalChat()).toBe(false);
		expect(navigation.params.get("conversation")).toBe("c1");
	});

	async function openActivity(sessionId: string, title: string) {
		await act(async () => {
			useChatActivityStore.setState({
				entries: [
					{
						id: `flowpilot:${sessionId}`,
						target: { kind: "flowpilot", sessionId },
						messageId: "m-reply",
						title,
						preview: "The report is ready.",
						createdAt: Date.now(),
					},
				],
			});
		});
		await click(
			container.querySelector<HTMLElement>(
				"button[aria-label='Conversation updates']",
			),
		);
		await click(
			Array.from(window.document.body.querySelectorAll("button")).find(
				(button) => button.textContent?.includes(title),
			),
		);
		await settle();
	}

	test("activity from a bot's chat opens that bot's chat over the open room", async () => {
		await openActivity("bot:bot_scout", "Scout replied about the digest");
		expect(restoreConversation).toHaveBeenCalledWith("bot:bot_scout");
		expect(showsGlobalChat()).toBe(true);
		expect(dockTitle()).toBe("Scout");
		expect(navigation.replace).toHaveBeenLastCalledWith("/bots?bot=bot_scout", {
			scroll: false,
		});
	});

	test("FlowPilot activity opened from the bell opens Flux's chat", async () => {
		await openActivity("chat-1", "Weekly report");
		expect(restoreConversation).toHaveBeenCalledWith("chat-1");
		expect(activeConversation()).toBe("chat-1");
		expect(dockTitle()).toBe("Flux");
		expect(navigation.replace).toHaveBeenLastCalledWith("/bots?bot=flux", {
			scroll: false,
		});
	});

	test("activity reopens a closed Flux dock on the selected session and updates its bookmark", async () => {
		expect(new URLSearchParams(window.location.search).get("session")).toBe(
			"chat-1",
		);
		await click(
			container.querySelector<HTMLElement>(
				"aside button[aria-label='Close chat']",
			),
		);
		expect(container.querySelector("aside")).toBeNull();
		await openActivity("chat-activity", "Another report is ready");
		expect(activeConversation()).toBe("chat-activity");
		expect(showsGlobalChat()).toBe(true);
		expect(new URLSearchParams(window.location.search).get("session")).toBe(
			"chat-activity",
		);
	});

	test("a draft is FlowPilot's: queued while a bot's chat is closed, it opens Flux's chat", async () => {
		await click(buttonContaining("Scout", "Runs in the cloud"));
		await click(
			container.querySelector<HTMLElement>(
				"aside button[aria-label='Close chat']",
			),
		);
		expect(container.querySelector("aside")).toBeNull();
		await act(async () => {
			useGlobalChatStore.getState().setDraft({ prompt: "Plan my week" });
		});
		await settle();
		expect(showsGlobalChat()).toBe(true);
		expect(dockTitle()).toBe("Flux");
		expect(activeConversation().startsWith("bot:")).toBe(false);
		expect(navigation.replace).toHaveBeenLastCalledWith("/bots?bot=flux", {
			scroll: false,
		});
		useGlobalChatStore.setState({ draft: null, nativeDrafts: [] });
	});

	test("a closed Flux chat reopens for a draft without touching the URL", async () => {
		await click(
			container.querySelector<HTMLElement>(
				"aside button[aria-label='Close chat']",
			),
		);
		const urlWrites = navigation.replace.mock.calls.length;
		await act(async () => {
			useGlobalChatStore.getState().setDraft({ prompt: "Plan my week" });
		});
		await settle();
		expect(showsGlobalChat()).toBe(true);
		expect(navigation.replace.mock.calls.length).toBe(urlWrites);
		useGlobalChatStore.setState({ draft: null, nativeDrafts: [] });
	});

	test("a draft written to a bot opens that bot's chat, not Flux's", async () => {
		await click(
			container.querySelector<HTMLElement>(
				"aside button[aria-label='Close chat']",
			),
		);
		await act(async () => {
			useGlobalChatStore
				.getState()
				.setDraft({ prompt: "Summarize the launch", botId: "bot_scout" });
		});
		await settle();
		expect(showsGlobalChat()).toBe(true);
		expect(dockTitle()).toBe("Scout");
		expect(navigation.replace).toHaveBeenLastCalledWith("/bots?bot=bot_scout", {
			scroll: false,
		});
		useGlobalChatStore.setState({ draft: null, nativeDrafts: [] });
	});

	test("a copy made with Duplicate in the settings opens in the dock", async () => {
		const copy = bot("bot_scout_copy", { name: "Scout copy" });
		// The clone mutation has already put the copy in the list.
		client.setQueryData(botKeys.list(), [...bots, copy]);
		const editor = renderEditor.mock.calls.at(-1)?.[0];
		expect(editor?.onDuplicated).toBeDefined();
		await act(async () => editor?.onDuplicated?.(copy));
		await settle();
		expect(navigation.replace).toHaveBeenLastCalledWith(
			"/bots?bot=bot_scout_copy",
			{ scroll: false },
		);
		expect(activeConversation()).toBe("bot:bot_scout_copy");
		expect(dockTitle()).toBe("Scout copy");
	});
});
