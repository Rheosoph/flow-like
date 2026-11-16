import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import type { IBot } from "../../state/backend-state/bot-state";
import type {
	IConversation,
	IInboxEntry,
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

const navigation = {
	params: new URLSearchParams(),
	replace: mock((href: string) => {
		navigation.params = new URLSearchParams(href.split("?")[1] ?? "");
		window.history.replaceState(null, "", href);
	}),
};
const editorProps: { open: boolean; botId?: string | null }[] = [];

const CHAT_BODY = "../global-chat/global-chat-body";
const EDITOR = "./editor";
const BOT_EXPRESSION = "../../hooks/use-bot-expression";

const actual = {
	locales: { ...(await import("@flow-like/locales")) },
	navigation: { ...(await import("next/navigation")) },
	oidc: { ...(await import("react-oidc-context")) },
	chatBody: { ...(await import(CHAT_BODY)) },
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
	useAuth: () => ({ isAuthenticated: false, user: null }),
}));
mock.module(CHAT_BODY, () => ({
	...actual.chatBody,
	GlobalChatBody: () =>
		createElement("div", { "data-testid": "global-chat" }, "Global chat"),
}));
mock.module(EDITOR, () => ({
	...actual.editor,
	BotEditorDialog: (props: { open: boolean; botId?: string | null }) => {
		editorProps.push({ open: props.open, botId: props.botId });
		return null;
	},
}));

const { createRoot } = await import("react-dom/client");
const { QueryClient, QueryClientProvider } = await import(
	"@tanstack/react-query"
);
const { BackendContext, useAuthStatusStore, useBackendStore } = await import(
	"../../state/backend-state"
);
const { useGlobalChatStore } = await import(
	"../../state/global-chat/global-chat-store"
);
const { BotsView } = await import("./bots-view");

const SCOUT = "bot_scoutlocal000000001";
const WRITER = "bot_writerlocal00000001";
const now = new Date().toISOString();

function localBot(id: string, name: string): IBot {
	return {
		id,
		owner_id: "local",
		system: null,
		preset_id: null,
		name,
		description: `${name} lives here`,
		instructions: null,
		appearance: { kind: "mochi" },
		model: { primary: null, fallbacks: [] },
		classification: { pii: false },
		capabilities: ["internet"],
		profile_id: null,
		location: "desktop",
		device_id: null,
		created_at: now,
		updated_at: now,
		local: true,
	};
}

function member(principal_id: string, kind: "user" | "bot" = "bot") {
	return { principal_id, kind, added_by: "local", joined_at: now };
}

function message(id: string, sender: string, text: string, minute: number) {
	const time = Date.now() - (60 - minute) * 60_000;
	return {
		id: `e_${id}`,
		message_id: id,
		conversation_id: "local-direct",
		sender_id: sender,
		kind: "message",
		envelope: { v: 1, enc: "none", body: { text } },
		created_at: new Date(time).toISOString(),
		cursor: `${time * 1000}:e_${id}`,
	} satisfies IInboxEntry;
}

const entries = [
	message("m2", SCOUT, "Hello! I live on this computer.", 2),
	message("m1", "local", "Hi Scout", 1),
];

const direct: IConversation = {
	id: "local-direct",
	kind: "direct",
	title: null,
	encryption: "plain",
	created_by: "local",
	created_at: now,
	last_activity_at: now,
	members: [member("local", "user"), member(SCOUT)],
	unread_count: 0,
	last_entry: entries[1] ?? null,
	local: true,
};

const room: IConversation = {
	...direct,
	id: "local-room",
	kind: "group",
	title: "Weekend plans",
	members: [member("local", "user"), member(SCOUT), member(WRITER)],
	last_entry: null,
};

const lookupUser = mock(
	async (id: string): Promise<IUserLookup> => ({
		id,
		name: id === "local" ? "You" : id,
		created_at: "",
	}),
);
const lookupUsers = mock(async (_ids: string[]): Promise<IUserLookup[]> => []);
const markRead = mock(async () => ({ last_read_at: now }));

const localHost = {
	checkMove: mock(async () => {
		throw new Error("Moving needs an account");
	}),
	move: mock(async () => {
		throw new Error("Moving needs an account");
	}),
};

const backend = {
	appState: { getApps: async () => [] },
	botState: {
		local: localHost,
		listBots: async () => [
			localBot(SCOUT, "Scout"),
			localBot(WRITER, "Writer"),
		],
	},
	inboxState: {
		listConversations: async () => [direct, room],
		getConversation: async (id: string) => (id === room.id ? room : direct),
		listEntries: async () => ({ entries, next_before: null }),
		markRead,
	},
	userState: {
		getInfo: async () => {
			throw new Error("Signed out");
		},
		getProfile: async () => null,
		getSettingsProfile: async () => ({
			hub_profile: { id: "profile_1", bits: [] },
		}),
		lookupUser,
		lookupUsers,
	},
};

const previousBackend = useBackendStore.getState().backend;
const previousSignedIn = useAuthStatusStore.getState().signedIn;
const previousChat = useGlobalChatStore.getState();
const client = new QueryClient({
	defaultOptions: { queries: { retry: false } },
});
let root: ReturnType<typeof createRoot>;
let container: HTMLElement;

function buttonContaining(...texts: string[]) {
	return Array.from(window.document.body.querySelectorAll("button")).find(
		(button) => texts.every((text) => button.textContent?.includes(text)),
	) as HTMLButtonElement | undefined;
}

function lookOf(element?: Element) {
	return element?.querySelector("[data-kind]")?.getAttribute("data-kind");
}

function dockText() {
	return container.querySelector("aside")?.textContent ?? "";
}

async function click(element: { click(): void } | null | undefined) {
	expect(element ?? undefined).toBeDefined();
	await act(async () => {
		element?.click();
	});
	await settle();
}

/** Ids the directory was asked about, one per call argument. */
function lookedUp(): string[] {
	return [
		...lookupUser.mock.calls.map(([id]) => id),
		...lookupUsers.mock.calls.flatMap(([ids]) => ids),
	];
}

beforeAll(async () => {
	installDomGlobals();
	useAuthStatusStore.getState().setSignedIn(false);
	useBackendStore.getState().setBackend(backend as never);
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	await act(async () =>
		root.render(
			createElement(QueryClientProvider, { client }, createElement(BotsView)),
		),
	);
	await settle();
	await settle(100);
});

afterAll(async () => {
	await act(async () => root.unmount());
	client.clear();
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
	mock.module(EDITOR, () => actual.editor);
	mock.module(BOT_EXPRESSION, () => actual.expression);
	restoreDomGlobals();
});

describe("BotsView signed out on the desktop", () => {
	test("shows Assistant, Flux and the bots that live on this computer", () => {
		const text = container.textContent ?? "";
		expect(text).toContain(
			"Your bots and chats stay on this computer. Sign in to talk with people.",
		);
		expect(text).toContain("Assistant");
		expect(text).toContain("Flux");
		expect(lookOf(buttonContaining("Assistant", "Built in"))).toBe("pip");
		expect(lookOf(buttonContaining("Flux", "Built in"))).toBe("flux");
		expect(text).toContain("Scout lives here");
		expect(text).toContain("Writer lives here");
		expect(text).toContain("4 bots");
		expect(text.split("Local only").length - 1).toBe(2);
		expect(text.split("Built in").length - 1).toBe(2);
		expect(text).not.toContain("Sign in to create bots");
		expect(text).not.toContain("live only on this computer");
	});

	test("the chat's pill shows the built-in bot you opened", async () => {
		const chatBot = () =>
			container.querySelector("aside [data-kind]")?.getAttribute("data-kind");
		expect(
			container.querySelector("aside [data-testid='global-chat']"),
		).not.toBeNull();
		expect(chatBot()).toBe("pip");
		await click(buttonContaining("Flux", "Built in"));
		expect(chatBot()).toBe("flux");
	});

	test("New bot creates a bot on this computer", async () => {
		const header = Array.from(container.querySelectorAll("header button")).find(
			(button) => button.textContent === "New bot",
		) as HTMLButtonElement | undefined;
		expect(header?.disabled).toBe(false);
		await click(header);
		expect(editorProps.at(-1)).toEqual({ open: true, botId: null });
		const tile = buttonContaining("New bot", "Describe what it should do");
		expect(tile?.disabled).toBe(false);
	});

	test("lists the chats on this computer with you as `local`", () => {
		const text = container.textContent ?? "";
		expect(text).toContain("Weekend plans");
		expect(text).toContain("1 person · 2 bots");
		expect(text).toContain("You: Hi Scout");
		expect(
			container.querySelectorAll(
				"[aria-label='This chat lives only on this computer.']",
			).length,
		).toBe(2);
		expect(
			container.querySelector("[aria-label='Scout, Your bot']"),
		).not.toBeNull();
	});

	test("a local bot opens its own chat, says where it lives and offers no move while signed out", async () => {
		await click(buttonContaining("Scout", "Runs in the desktop app"));
		expect(
			container.querySelector("aside [data-testid='global-chat']"),
		).not.toBeNull();
		expect(useGlobalChatStore.getState().activeConversationId).toBe(
			`bot:${SCOUT}`,
		);
		const pill = container.querySelector<HTMLButtonElement>(
			"aside [data-slot='bot-pill']",
		);
		expect(
			pill?.querySelector("[data-slot='bot-pill-name']")?.textContent,
		).toBe("Scout");
		expect(pill?.textContent).toContain("Local only");
		expect(lookOf(pill ?? undefined)).toBe("mochi");
		await click(pill);
		const details =
			container.querySelector("aside section[aria-label='About Scout']")
				?.textContent ?? "";
		expect(details).toContain("Scout lives here");
		expect(details).toContain("This computer");
		expect(details).toContain("Not connected to your account");
		expect(details).toContain(
			"This bot is saved only on this computer. Bots in your account can't find or message it.",
		);
		expect(details).toContain("Internet");
		expect(details).not.toContain("Move to your account");
		expect(details).not.toContain("runtime is live");
		await click(
			container.querySelector<HTMLElement>(
				"aside button[aria-label='Back to the chat']",
			),
		);
	});

	test("the Assistant without an account row answers as `general`", async () => {
		await click(buttonContaining("Assistant", "Built in"));
		expect(useGlobalChatStore.getState().activeConversationId).toBe(
			"bot:general",
		);
		expect(
			container.querySelector("aside [data-slot='bot-pill-name']")?.textContent,
		).toBe("Assistant");
	});

	test("a bot's direct message opens as the bot's own chat on this computer", async () => {
		await click(buttonContaining("Scout", "You: Hi Scout"));
		const dock = dockText();
		expect(dock).toContain("Scout");
		expect(dock).toContain("Local only");
		expect(dock).not.toContain("Direct message");
		await click(
			container.querySelector<HTMLElement>(
				"aside button[aria-label='Close chat']",
			),
		);
	});

	test("never asks the directory about bots on this computer", () => {
		expect(lookedUp().filter((id) => id.startsWith("bot_"))).toEqual([]);
		expect(lookedUp().every((id) => id === "local")).toBe(true);
	});

	test("New message offers your bots here and no people", async () => {
		await click(buttonContaining("New message"));
		const dialog = window.document.body.querySelector("[role='dialog']");
		const text = dialog?.textContent ?? "";
		expect(text).toContain("Your bots");
		expect(text).toContain("Scout");
		expect(text).toContain("Writer");
		expect(text).not.toContain("People");
		expect(text).toContain(
			"Signed out, you can talk with your bots on this computer. Sign in to talk with people.",
		);
		expect(
			dialog?.querySelector("input[aria-label='Search your bots']"),
		).not.toBeNull();
		await act(async () => {
			window.document.dispatchEvent(
				new window.KeyboardEvent("keydown", {
					key: "Escape",
					bubbles: true,
				}) as never,
			);
		});
		await settle();
	});
});

describe("BotsView signed out where bots can't live on the device", () => {
	test("keeps Assistant and Flux only, as before", async () => {
		const host = window.document.createElement("div") as unknown as HTMLElement;
		window.document.body.appendChild(host as never);
		const guestRoot = createRoot(host);
		const web = {
			...backend,
			botState: { listBots: backend.botState.listBots },
		};
		await act(async () =>
			guestRoot.render(
				createElement(
					QueryClientProvider,
					{ client: new QueryClient() },
					createElement(
						BackendContext.Provider,
						{ value: web as never },
						createElement(BotsView),
					),
				),
			),
		);
		await settle();
		const text = host.textContent ?? "";
		expect(text).toContain(
			"Sign in to create bots and talk with people. Assistant and Flux work without an account.",
		);
		expect(text).not.toContain("Scout");
		expect(text).not.toContain("Rooms and people");
		const newBot = Array.from(host.querySelectorAll("header button")).find(
			(button) => button.textContent === "New bot",
		) as HTMLButtonElement | undefined;
		expect(newBot?.disabled).toBe(true);
		await act(async () => guestRoot.unmount());
		host.remove();
	});
});
