import {
	afterAll,
	afterEach,
	beforeAll,
	describe,
	expect,
	mock,
	spyOn,
	test,
} from "bun:test";
import { act, createElement } from "react";
import type { IBot } from "../../state/backend-state/bot-state";
import type { IConversation } from "../../state/backend-state/inbox-state";
import type { GlobalToolPrompt } from "../../state/global-chat/global-chat-store";
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
const session = { signedIn: true };
const editor: { botId?: string | null; open?: boolean } = {};

const CHAT_BODY = "../global-chat/global-chat-body";
const CHAT_STREAM = "../../state/global-chat/global-chat-stream";
const EDITOR = "./editor";
const BOT_EXPRESSION = "../../hooks/use-bot-expression";

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
// Like the real restore, the conversation comes on screen (no IndexedDB here).
const restoreConversation = mock(async (conversationId: string) => {
	useGlobalChatStore.getState().loadConversation(conversationId, []);
	return true;
});
mock.module(CHAT_STREAM, () => ({
	...actual.chatStream,
	restoreGlobalChatConversation: restoreConversation,
}));
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
	useAuth: () =>
		session.signedIn
			? {
					isAuthenticated: true,
					user: { access_token: "token", profile: { sub: "me" } },
				}
			: { isAuthenticated: false, user: null },
}));
mock.module(CHAT_BODY, () => ({
	...actual.chatBody,
	GlobalChatBody: () =>
		createElement(
			"div",
			{
				"data-testid": "global-chat",
				"data-loading": String(
					useGlobalChatStore((state) => state.conversationLoading),
				),
			},
			"Global chat",
		),
}));
mock.module(EDITOR, () => ({
	...actual.editor,
	BotEditorDialog: (props: { open: boolean; botId?: string | null }) => {
		editor.open = props.open;
		editor.botId = props.botId;
		return null;
	},
}));

const { createRoot } = await import("react-dom/client");
const { QueryClient, QueryClientProvider } = await import(
	"@tanstack/react-query"
);
const { useAuthStatusStore, useBackendStore } = await import(
	"../../state/backend-state"
);
const { useGlobalChatStore } = await import(
	"../../state/global-chat/global-chat-store"
);
const { globalChatDb } = await import("../../state/global-chat/global-chat-db");
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
		created_at: "2026-11-01T10:00:00Z",
		updated_at: "2026-11-01T10:00:00Z",
		...overrides,
	};
}

const ACCOUNT_ASSISTANT = "bot_general";
/** Chatted with while signed out, so it stays beside the account's. */
const LOCAL_ASSISTANT = "bot_generallocal0000001";

const accountBots = [
	bot(ACCOUNT_ASSISTANT, { system: "general" }),
	bot("bot_flux", { system: "flux" }),
	bot("bot_scout", { name: "Scout" }),
];
const localAssistant = bot(LOCAL_ASSISTANT, {
	system: "general",
	owner_id: "local",
	location: "desktop",
	local: true,
	updated_at: "2026-11-02T09:00:00Z",
});

const previousBackend = useBackendStore.getState().backend;
const previousSignedIn = useAuthStatusStore.getState().signedIn;
const previousChat = useGlobalChatStore.getState();
let root: ReturnType<typeof createRoot> | undefined;
let container: HTMLElement;

async function mount(
	listBots: () => Promise<IBot[]>,
	signedIn = true,
	listConversations: () => Promise<IConversation[]> = async () => [],
) {
	session.signedIn = signedIn;
	navigation.params = new URLSearchParams();
	window.history.replaceState(null, "", "/bots");
	useAuthStatusStore.getState().setSignedIn(signedIn);
	useBackendStore.getState().setBackend({
		appState: { getApps: async () => [] },
		botState: {
			listBots,
			local: { checkMove: async () => ({}), move: async () => ({}) },
		},
		inboxState: { listConversations },
		userState: {
			getProfile: async () => null,
			getSettingsProfile: async () => ({
				hub_profile: { id: "profile_1", bits: [] },
			}),
			getInfo: async () => ({ id: "me", name: "Felix Schultz" }),
			lookupUser: async () => null,
			lookupUsers: async () => [],
		},
	} as never);
	useGlobalChatStore.setState({
		activeConversationId: "flux-fresh",
		messages: [],
		runs: {},
		queue: [],
		draft: null,
		nativeDrafts: [],
		toolPrompt: null,
	});
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	const view = createElement(
		QueryClientProvider,
		{ client },
		createElement(BotsView),
	);
	await act(async () => root?.render(view));
	await settle();
	await settle(60);
	return client;
}

beforeAll(() => {
	installDomGlobals();
});

afterEach(async () => {
	await act(async () => root?.unmount());
	root = undefined;
	container.remove();
});

afterAll(async () => {
	useBackendStore.setState({ backend: previousBackend });
	useAuthStatusStore.setState({ signedIn: previousSignedIn });
	useGlobalChatStore.setState(previousChat, true);
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

function card(local: boolean) {
	return Array.from(container.querySelectorAll("button")).find(
		(button) =>
			button.textContent?.includes("Assistant") &&
			button.textContent.includes("Built in") &&
			button.textContent.includes("On this device") === local,
	);
}

function currentCards() {
	return Array.from(
		container.querySelectorAll("button[aria-current='true']"),
	).map((button) =>
		button.textContent?.includes("On this device") ? "local" : "account",
	);
}

function activeConversation() {
	return useGlobalChatStore.getState().activeConversationId;
}

function dockButton(label: string) {
	return container.querySelector<HTMLButtonElement>(
		`aside button[aria-label='${label}']`,
	);
}

function panel() {
	return container.querySelector("aside section[aria-label^='About']");
}

function panelButton(label: string) {
	return Array.from(panel()?.querySelectorAll("button") ?? []).find(
		(button) =>
			button.textContent === label ||
			button.getAttribute("aria-label") === label,
	);
}

/** Checks as booleans, so a failure doesn't print a whole DOM tree. */
function panelOpen() {
	return panel() !== null;
}

function hasPanelButton(label: string) {
	return panelButton(label) !== undefined;
}

async function openPanel() {
	await click(
		container.querySelector<HTMLButtonElement>("aside [data-slot='bot-pill']"),
	);
	expect(panelOpen()).toBe(true);
}

function dockText() {
	return container.querySelector("aside")?.textContent ?? "";
}

async function click(element: { click(): void } | null | undefined) {
	expect(element ?? undefined).toBeDefined();
	await act(async () => element?.click());
	await settle();
}

describe("the Assistant on this computer beside the account's", () => {
	test("each opens its own chat, settings and move, and only the open one is highlighted", async () => {
		await mount(async () => [...accountBots, localAssistant]);
		expect(currentCards()).toEqual(["account"]);
		expect(activeConversation()).toBe(`bot:${ACCOUNT_ASSISTANT}`);

		await click(card(true));
		expect(navigation.params.get("bot")).toBe(LOCAL_ASSISTANT);
		expect(activeConversation()).toBe(`bot:${LOCAL_ASSISTANT}`);
		expect(currentCards()).toEqual(["local"]);
		await openPanel();
		expect(panel()?.textContent).toContain("On this device");
		expect(panelButton("Move to your account")).toBeDefined();
		await click(panelButton("Edit"));
		expect(editor).toEqual({ open: true, botId: LOCAL_ASSISTANT });

		await click(card(false));
		expect(navigation.params.get("bot")).toBeNull();
		expect(navigation.params.get("panel")).toBeNull();
		expect(activeConversation()).toBe(`bot:${ACCOUNT_ASSISTANT}`);
		expect(currentCards()).toEqual(["account"]);
		await openPanel();
		expect(hasPanelButton("Move to your account")).toBe(false);
		await click(panelButton("Edit"));
		expect(editor).toEqual({ open: true, botId: ACCOUNT_ASSISTANT });
	});

	test("showing the local one's chat opens its dock and leaves its conversation on screen", async () => {
		await mount(async () => [...accountBots, localAssistant]);
		await act(async () => {
			useGlobalChatStore
				.getState()
				.loadConversation(`bot:${LOCAL_ASSISTANT}`, []);
			useGlobalChatStore.getState().requestReveal();
		});
		await settle();
		expect(navigation.params.get("bot")).toBe(LOCAL_ASSISTANT);
		expect(activeConversation()).toBe(`bot:${LOCAL_ASSISTANT}`);
		expect(currentCards()).toEqual(["local"]);
	});
});

describe("switching to another bot's chat", () => {
	/** Holds every read of saved messages until `release`. */
	function holdReads() {
		let release: () => void = () => {};
		const held = new Promise<void>((resolve) => {
			release = resolve;
		});
		const spy = spyOn(globalChatDb.messages, "where").mockImplementation(
			(() => ({
				equals: () => ({ sortBy: async () => held.then(() => []) }),
			})) as never,
		);
		return { release, restore: () => spy.mockRestore() };
	}

	const chatLoading = () =>
		container
			.querySelector("[data-testid='global-chat']")
			?.getAttribute("data-loading");

	function botCard(name: string) {
		return Array.from(container.querySelectorAll("button")).find((button) =>
			button.textContent?.includes(name),
		);
	}

	test("the chat shows its loading state from the moment the dock switches until the bot's conversation lands", async () => {
		await mount(async () => accountBots);
		expect(chatLoading()).toBe("false");
		const reads = holdReads();
		try {
			await click(botCard("Scout"));
			expect(chatLoading()).toBe("true");
			expect(activeConversation()).toBe(`bot:${ACCOUNT_ASSISTANT}`);

			await act(async () => reads.release());
			await settle();
			expect(chatLoading()).toBe("false");
			expect(activeConversation()).toBe("bot:bot_scout");
		} finally {
			reads.restore();
		}
	});
});

describe("Needs you", () => {
	const crew = [...accountBots, bot("bot_writer", { name: "Writer" })];

	function waitingCard() {
		return Array.from(container.querySelectorAll("article")).find((article) =>
			article.textContent?.includes("Waiting for your answer"),
		);
	}

	/** A run of `conversationId` asks; without one, no run owns the prompt. */
	async function asks(
		conversationId?: string,
		kind: GlobalToolPrompt["kind"] = "approval",
	) {
		const runId = conversationId && `run-${conversationId}`;
		await act(async () => {
			useGlobalChatStore.setState({
				runs: runId ? { [runId]: { runId, conversationId } as never } : {},
				toolPrompt: {
					id: `prompt-${conversationId ?? "unowned"}`,
					kind,
					runId: runId || undefined,
					conversationId,
					toolName: kind === "ask" ? "ask_user" : "call_app_event",
					title:
						kind === "ask" ? "Which project?" : "Approve app event execution",
					respond: () => {},
				},
			});
		});
		await settle();
	}

	const scoutAsks = () => asks("bot:bot_scout");

	function cardButton(name: string) {
		return Array.from(container.querySelectorAll("button")).find(
			(button) =>
				button.textContent?.includes(name) &&
				button.textContent.includes("Built in"),
		);
	}

	function openChatIn(card: Element | undefined) {
		return Array.from(card?.querySelectorAll("button") ?? []).find(
			(button) => button.textContent === "Open chat",
		);
	}

	test("an inbox approval follows the visibility of its bot chat until answered", async () => {
		const at = "2026-11-04T10:00:00Z";
		const conversation: IConversation = {
			id: "scout-owner-chat",
			kind: "direct",
			title: null,
			encryption: "plain",
			created_by: "me",
			created_at: at,
			last_activity_at: at,
			members: [
				{ principal_id: "me", kind: "user", added_by: null, joined_at: at },
				{
					principal_id: "bot_scout",
					kind: "bot",
					added_by: "me",
					joined_at: at,
				},
			],
			unread_count: 0,
			last_entry: {
				id: "approval-entry",
				message_id: "approval-scout",
				conversation_id: "scout-owner-chat",
				sender_id: "bot_scout",
				kind: "message",
				created_at: at,
				cursor: "1000:approval-entry",
				envelope: {
					v: 1,
					enc: "none",
					body: {
						text: "Approve sending the digest",
						approval: {
							id: "approval-scout",
							turn_id: "turn-scout",
							tool: "send_message",
							summary: "Send the digest",
							scope: "",
						},
					},
				},
			},
		};
		let listed = [conversation];
		const client = await mount(
			async () => crew,
			true,
			async () => listed,
		);
		const approvalCard = () =>
			Array.from(container.querySelectorAll("article")).find((article) =>
				article.textContent?.includes("Approve sending the digest"),
			);
		expect(activeConversation()).toBe(`bot:${ACCOUNT_ASSISTANT}`);
		expect(approvalCard()).toBeDefined();

		await click(openChatIn(approvalCard()));
		expect(navigation.params.get("conversation")).toBe(conversation.id);
		expect(activeConversation()).toBe("bot:bot_scout");
		expect(approvalCard()).toBeUndefined();

		await openPanel();
		expect(approvalCard()).toBeDefined();
		await click(openChatIn(approvalCard()));
		expect(panelOpen()).toBe(false);
		expect(approvalCard()).toBeUndefined();

		await click(dockButton("Close chat"));
		expect(approvalCard()).toBeDefined();
		await act(async () => {
			if (!conversation.last_entry) throw new Error("Missing approval fixture");
			listed = [
				{
					...conversation,
					last_entry: {
						...conversation.last_entry,
						id: "answer-entry",
						message_id: "answer-scout",
						sender_id: "me",
						cursor: "2000:answer-entry",
						envelope: {
							v: 1,
							enc: "none",
							body: {
								text: "",
								approval_response: {
									approval_id: "approval-scout",
									decision: "deny",
								},
							},
						},
					},
				},
			];
			await client.invalidateQueries({ queryKey: inboxKeys.conversations() });
		});
		await settle();
		expect(approvalCard()).toBeUndefined();
	});

	test("names the bot whose run asks and opens its chat", async () => {
		await mount(async () => crew);
		await click(
			Array.from(container.querySelectorAll("button")).find((button) =>
				button.textContent?.includes("Writer"),
			),
		);
		expect(activeConversation()).toBe("bot:bot_writer");
		await click(dockButton("Close chat"));
		await scoutAsks();

		const card = waitingCard();
		expect(card?.textContent).toContain("Scout");
		expect(card?.textContent).not.toContain("Writer");
		await click(
			Array.from(card?.querySelectorAll("button") ?? []).find(
				(button) => button.textContent === "Open chat",
			),
		);
		expect(navigation.params.get("bot")).toBe("bot_scout");
		expect(activeConversation()).toBe("bot:bot_scout");
		expect(waitingCard()).toBeUndefined();
	});

	test("stays up while another bot's chat is on screen", async () => {
		await mount(async () => crew);
		await click(
			Array.from(container.querySelectorAll("button")).find((button) =>
				button.textContent?.includes("Writer"),
			),
		);
		await scoutAsks();
		expect(waitingCard()?.textContent).toContain("Scout");
	});

	test("a Flux chat's question opens that Flux chat, and only that one hides it", async () => {
		await mount(async () => crew);
		await click(cardButton("Flux"));
		const openFluxChat = activeConversation();
		expect(openFluxChat.startsWith("bot:")).toBe(false);
		await asks("flux-launch", "ask");

		const card = waitingCard();
		expect(card?.textContent).toContain("Flux");
		await click(openChatIn(card));
		expect(restoreConversation).toHaveBeenLastCalledWith("flux-launch");
		expect(activeConversation()).toBe("flux-launch");
		expect(navigation.params.get("bot")).toBe("flux");
		expect(waitingCard()).toBeUndefined();
	});

	test.each(["ask", "approval"] as const)(
		"opening a pending %s hides it until the chat closes, and resolving it removes it",
		async (kind) => {
			await mount(async () => crew);
			await asks("bot:bot_scout", kind);
			const prompt = useGlobalChatStore.getState().toolPrompt;
			expect(waitingCard()?.textContent).toContain("Scout");

			await click(openChatIn(waitingCard()));
			expect(activeConversation()).toBe("bot:bot_scout");
			expect(waitingCard()).toBeUndefined();
			expect(useGlobalChatStore.getState().toolPrompt).toBe(prompt);

			await click(dockButton("Close chat"));
			expect(waitingCard()?.textContent).toContain("Scout");
			await click(openChatIn(waitingCard()));

			// The tool bridge clears the prompt after an answer, denial or cancellation.
			await act(async () => {
				useGlobalChatStore.getState().setToolPrompt(null);
			});
			await click(dockButton("Close chat"));
			expect(waitingCard()).toBeUndefined();

			await click(
				Array.from(container.querySelectorAll("button")).find((button) =>
					button.textContent?.includes("Scout"),
				),
			);
			expect(activeConversation()).toBe("bot:bot_scout");
			await click(dockButton("Close chat"));
			expect(waitingCard()).toBeUndefined();
		},
	);

	test("a prompt no run owns is the open chat's, as before", async () => {
		await mount(async () => crew);
		await click(
			Array.from(container.querySelectorAll("button")).find((button) =>
				button.textContent?.includes("Writer"),
			),
		);
		await asks(undefined);
		expect(waitingCard()).toBeUndefined();
		await click(dockButton("Close chat"));
		expect(waitingCard()?.textContent).toContain("Writer");
	});

	test("while the panel covers the asking chat the card stays up, and it opens the chat again", async () => {
		await mount(async () => crew);
		await openPanel();
		await asks(`bot:${ACCOUNT_ASSISTANT}`);
		const card = waitingCard();
		expect(card?.textContent).toContain("Assistant");

		await click(openChatIn(card));
		expect(panelOpen()).toBe(false);
		expect(navigation.params.get("panel")).toBeNull();
		expect(waitingCard()).toBeUndefined();
	});
});

describe("where the panel opens", () => {
	let original: HTMLElement["getBoundingClientRect"];

	beforeAll(() => {
		original = HTMLElement.prototype.getBoundingClientRect;
	});

	afterEach(() => {
		HTMLElement.prototype.getBoundingClientRect = original;
	});

	/** The Bots tab measures its own width once it mounts. */
	async function mountAt(width: number) {
		HTMLElement.prototype.getBoundingClientRect = () => ({ width }) as DOMRect;
		await mount(async () => accountBots);
	}

	function chatCovered() {
		return (
			container
				.querySelector("aside [data-testid='global-chat']")
				?.closest("[inert]") !== null
		);
	}

	test("a wide tab opens it beside the chat and gives the dock its width", async () => {
		await mountAt(1400);
		const aside = container.querySelector("aside");
		expect(aside?.style.width).toBe("420px");
		await openPanel();
		expect(aside?.style.width).toBe("820px");
		expect(chatCovered()).toBe(false);
		expect(hasPanelButton("Close details")).toBe(true);
		expect(hasPanelButton("Back to the chat")).toBe(false);
		expect(hasPanelButton("Close chat")).toBe(false);

		await click(panelButton("Close details"));
		expect(panelOpen()).toBe(false);
		expect(aside?.style.width).toBe("420px");
	});

	test("a narrow tab lays it over the chat, with the way back and the dock's own close", async () => {
		await mountAt(900);
		await openPanel();
		expect(container.querySelector("aside")?.style.width).toBe("420px");
		expect(chatCovered()).toBe(true);
		expect(hasPanelButton("Back to the chat")).toBe(true);
		expect(hasPanelButton("Close chat")).toBe(true);
		expect(hasPanelButton("Close details")).toBe(false);
	});
});

describe("when the bot list can't be read", () => {
	for (const signedIn of [true, false]) {
		test(`${signedIn ? "signed in" : "on this computer"}, the Assistant waits for it instead of answering as \`general\``, async () => {
			let fail = true;
			await mount(async () => {
				if (fail) throw new Error("bots unavailable");
				return signedIn ? accountBots : [localAssistant];
			}, signedIn);
			expect(activeConversation()).not.toBe("bot:general");
			expect(dockText()).toContain("Couldn't load your bots");

			fail = false;
			const retry = Array.from(
				container.querySelectorAll<HTMLButtonElement>("aside button"),
			).find((button) => button.textContent === "Try again");
			await click(retry);
			await settle(60);
			expect(
				container.querySelector("aside [data-testid='global-chat']"),
			).not.toBeNull();
			expect(activeConversation()).toBe(
				`bot:${signedIn ? ACCOUNT_ASSISTANT : LOCAL_ASSISTANT}`,
			);
		});
	}
});
