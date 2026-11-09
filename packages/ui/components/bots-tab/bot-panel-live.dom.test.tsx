import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import type {
	IBot,
	IBotPermissions,
	IBotPresence,
	IBotTurn,
	IBotWorkspace,
} from "../../state/backend-state/bot-state";
import type {
	MemoryEntry,
	MemoryOwner,
} from "../../state/global-chat/global-chat-memory";
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

const CHAT_BODY = "../global-chat/global-chat-body";
const EDITOR = "./editor";
const BOT_EXPRESSION = "../../hooks/use-bot-expression";
const VIEWS = "./workspace-views";
const MEMORY = "../../state/global-chat/global-chat-memory";

interface StorageProps {
	appId: string;
	prefix: string;
	title?: string;
	canWrite?: boolean;
	fileToUrl: (file: string) => Promise<string>;
}

const shown = { storage: [] as StorageProps[], studio: [] as string[] };
const editor: { open?: boolean; botId?: string | null } = {};

const savedMemories: Record<string, MemoryEntry[]> = {
	bot_scout: [
		{
			id: "m1",
			content: "Prefers the digest on Mondays",
			role: "preference",
			timestamp: Date.now() - 120_000,
		},
		{
			id: "m2",
			content: "Works on the Q4 launch",
			role: "observation",
			timestamp: Date.now() - 7_200_000,
		},
	],
};
const memory = {
	list: mock(async (owner: MemoryOwner, _token?: string) => [
		...(savedMemories[owner.botId ?? ""] ?? []),
	]),
	remove: mock(
		async (_owner: MemoryOwner, _id: string, _token?: string) => undefined,
	),
	clear: mock(async (_owner: MemoryOwner, _token?: string) => undefined),
};

// bun keeps module mocks for every later file in the process, so each one spreads the real
// module and is put back in afterAll.
const actual = {
	locales: { ...(await import("@flow-like/locales")) },
	navigation: { ...(await import("next/navigation")) },
	oidc: { ...(await import("react-oidc-context")) },
	chatBody: { ...(await import(CHAT_BODY)) },
	editor: { ...(await import(EDITOR)) },
	expression: { ...(await import(BOT_EXPRESSION)) },
	views: { ...(await import(VIEWS)) },
	memory: { ...(await import(MEMORY)) },
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
		user: { access_token: "token", profile: { sub: "me" } },
	}),
}));
mock.module(CHAT_BODY, () => ({
	...actual.chatBody,
	GlobalChatBody: () =>
		createElement("div", { "data-testid": "global-chat" }, "Global chat"),
}));
mock.module(EDITOR, () => ({
	...actual.editor,
	BotEditorDialog: (props: { open: boolean; botId?: string | null }) => {
		editor.open = props.open;
		editor.botId = props.botId;
		return null;
	},
}));
mock.module(VIEWS, () => ({
	WorkspaceStorage: (props: StorageProps) => {
		shown.storage.push(props);
		return createElement(
			"div",
			{ "data-testid": "storage" },
			`Files of ${props.appId}`,
		);
	},
	WorkspaceDataStudio: ({ appId }: { appId: string }) => {
		shown.studio.push(appId);
		return createElement(
			"div",
			{ "data-testid": "data-studio" },
			`Tables of ${appId}`,
		);
	},
}));
mock.module(MEMORY, () => ({
	...actual.memory,
	listGlobalChatMemories: memory.list,
	deleteGlobalChatMemory: memory.remove,
	clearGlobalChatMemory: memory.clear,
}));

const { createRoot } = await import("react-dom/client");
const { QueryClient, QueryClientProvider, focusManager } = await import(
	"@tanstack/react-query"
);
const { useAuthStatusStore, useBackendStore } = await import(
	"../../state/backend-state"
);
const { useGlobalChatStore } = await import(
	"../../state/global-chat/global-chat-store"
);
const { botKeys } = await import("../../hooks/use-bots");
const { BOT_PRESENCE_POLL_MS } = await import("../../hooks/use-bot-presence");
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
	bot("bot_general", { system: "general", capabilities: ["memory"] }),
	bot("bot_flux", { system: "flux", capabilities: ["flowpilot"] }),
	bot("bot_scout", {
		name: "Scout",
		description: "Scout's job",
		location: "desktop",
	}),
	bot("bot_writer", { name: "Writer", description: "Writer's job" }),
	bot("bot_pebble", {
		name: "Pebble",
		description: "Pebble's job",
		owner_id: "local",
		location: "desktop",
		local: true,
	}),
	bot("bot_dash", {
		name: "Dash",
		description: "Dash's job",
		location: "device",
		device_id: "dev_1",
	}),
];

function grant(from: string, to: string) {
	return {
		id: `${from}>${to}`,
		from_bot_id: from,
		to_bot_id: to,
		granted_at: "2026-11-01T10:00:00Z",
		granted_by: "me",
	};
}

let permissions: IBotPermissions = {
	granted: [grant("bot_scout", "bot_writer")],
	received: [grant("bot_dash", "bot_scout")],
};
const listPermissions = mock(async (_botId: string) => permissions);
const revokePermission = mock(async (from: string, to: string) => {
	permissions = {
		granted: permissions.granted.filter(
			(item) => !(item.from_bot_id === from && item.to_bot_id === to),
		),
		received: permissions.received.filter(
			(item) => !(item.from_bot_id === from && item.to_bot_id === to),
		),
	};
});

let reports: Record<string, IBotPresence> = {
	bot_scout: { state: "online", last_seen_at: new Date().toISOString() },
};
const getPresence = mock(async () => reports);

const turnListeners = new Set<(turn: IBotTurn) => void>();
const cancelTurn = mock(async (_turn: IBotTurn) => {});
const turns = {
	list: mock(async (): Promise<IBotTurn[]> => []),
	subscribe: mock(async (listener: (turn: IBotTurn) => void) => {
		turnListeners.add(listener);
		return () => turnListeners.delete(listener);
	}),
	cancel: cancelTurn,
};
const emitTurn = (turn: Partial<IBotTurn> & Pick<IBotTurn, "id" | "status">) =>
	act(async () => {
		for (const listener of turnListeners) {
			listener({ bot_id: "bot_scout", conversation_id: "c1", ...turn });
		}
	});

const botState: Record<string, unknown> = {
	listBots: async () => bots,
	getWorkspace: async (botId: string): Promise<IBotWorkspace> => ({
		app_id: `ws-${botId}`,
	}),
	listPermissions,
	revokePermission,
	getPresence,
	turns,
};
const backend = {
	botState,
	appState: { getApps: async () => [] },
	inboxState: { listConversations: async () => [] },
	userState: {
		getInfo: async () => ({ id: "me", name: "Felix Schultz" }),
		lookupUser: async () => null,
		lookupUsers: async () => [],
		getSettingsProfile: async () => ({ hub_profile: { id: "profile-1" } }),
		getProfile: async () => null,
	},
	storageState: { downloadStorageItems: async () => [] },
	roleState: {
		getOwnRole: async () => ({
			permissions: 1,
			role_name: "Owner",
			is_owner: true,
			can_leave: false,
		}),
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

function view() {
	return createElement(
		QueryClientProvider,
		{ client },
		createElement(BotsView),
	);
}

beforeAll(async () => {
	installDomGlobals();
	window.history.replaceState(null, "", "/bots");
	useAuthStatusStore.getState().setSignedIn(true);
	useBackendStore.getState().setBackend(backend as never);
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
	await act(async () => root.render(view()));
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
	mock.module(VIEWS, () => actual.views);
	mock.module(MEMORY, () => actual.memory);
	restoreDomGlobals();
});

function dock() {
	return container.querySelector("aside");
}

function pill() {
	return dock()?.querySelector<HTMLButtonElement>("[data-slot='bot-pill']");
}

function panelText() {
	return (
		dock()?.querySelector("section[aria-label^='About']")?.textContent ?? ""
	);
}

function panelButton(label: string) {
	return Array.from(
		dock()
			?.querySelector("section[aria-label^='About']")
			?.querySelectorAll("button") ?? [],
	).find(
		(button) =>
			button.textContent === label ||
			button.getAttribute("aria-label") === label,
	);
}

function card(name: string) {
	return Array.from(container.querySelectorAll("button")).find(
		(button) =>
			button.querySelector("span.truncate")?.textContent === name &&
			button.closest(".group"),
	)?.textContent;
}

async function click(element: { click(): void } | null | undefined) {
	expect(element ?? undefined).toBeDefined();
	await act(async () => {
		element?.click();
	});
	await settle();
}

async function visit(query: string) {
	navigation.params = new URLSearchParams(query);
	window.history.replaceState(null, "", query ? `/bots?${query}` : "/bots");
	await act(async () => root.render(view()));
	await settle();
}

describe("what a bot may ask", () => {
	test("Details lists the bots it can ask and the bots that can ask it", async () => {
		await visit("bot=bot_scout&panel=details");
		expect(listPermissions).toHaveBeenCalledWith("bot_scout");
		const text = panelText();
		expect(text).toContain("Can ask");
		expect(text).toContain("Scout can ask");
		expect(text).toContain("Can ask Scout");
		expect(panelButton("Stop Scout asking Writer")).toBeDefined();
		expect(panelButton("Stop Dash asking Scout")).toBeDefined();
	});

	test("revoking a grant sends its own pair and the list drops it", async () => {
		await click(panelButton("Stop Dash asking Scout"));
		expect(revokePermission).toHaveBeenCalledTimes(1);
		expect(revokePermission).toHaveBeenLastCalledWith("bot_dash", "bot_scout");
		expect(panelButton("Stop Dash asking Scout")).toBeUndefined();

		await click(panelButton("Stop Scout asking Writer"));
		expect(revokePermission).toHaveBeenLastCalledWith(
			"bot_scout",
			"bot_writer",
		);
		expect(panelText()).toContain("No standing permissions");
	});
});

describe("whether a desktop or device bot is online", () => {
	test("a desktop bot reads the hub's report; its card, pill and Details agree", async () => {
		await visit("bot=bot_scout&panel=details");
		expect(card("Scout")).toContain("Online");
		expect(pill()?.textContent).toContain("Online");
		expect(panelText()).toContain("Status");
		expect(panelText()).toContain("Online");
	});

	test("never reported reads as not seen yet, and a stale report as offline with its last seen", async () => {
		expect(card("Dash")).toContain("Not seen yet");

		reports = {
			bot_scout: {
				state: "offline",
				last_seen_at: new Date(Date.now() - 3_600_000).toISOString(),
			},
		};
		await act(async () => {
			await client.invalidateQueries({ queryKey: botKeys.presence() });
		});
		await settle();
		expect(card("Scout")).toContain("Offline");
		expect(pill()?.textContent).toContain("Offline");
		expect(panelText()).toContain("Last seen");
		expect(panelText()).toContain("hour");
	});

	test("cloud bots read as online without a report, a bot on this device keeps the drive", async () => {
		expect(card("Writer")).toContain("Runs in the cloud");
		await visit("bot=bot_writer");
		expect(pill()?.querySelector("[class*='bg-primary']") !== null).toBe(true);
		await visit("bot=bot_pebble");
		expect(pill()?.textContent).toContain("On this device");
		expect(card("Pebble")).not.toContain("Online");
	});

	test("polls every minute and reads again when the window gets focus", async () => {
		const observers = client.getQueryCache().find({
			queryKey: botKeys.presence(),
		})?.observers;
		expect(observers?.length).toBeGreaterThan(0);
		for (const observer of observers ?? []) {
			expect(observer.options.refetchInterval).toBe(60_000);
			expect(observer.options.refetchOnWindowFocus).toBe("always");
		}
		expect(BOT_PRESENCE_POLL_MS).toBe(60_000);

		const before = getPresence.mock.calls.length;
		await act(async () => {
			focusManager.setFocused(false);
			focusManager.setFocused(true);
		});
		await settle();
		expect(getPresence.mock.calls.length).toBeGreaterThan(before);
	});
});

describe("a turn the bot works on without a chat window", () => {
	test("shows Working in Details and the pill while a turn runs, and Cancel ends it", async () => {
		await visit("bot=bot_scout&panel=details");
		expect(panelText()).not.toContain("Working");
		await emitTurn({ id: "bt_1", status: "running" });
		expect(panelText()).toContain("Working…");
		expect(pill()?.textContent).toContain("Working…");

		await click(panelButton("Cancel"));
		expect(cancelTurn).toHaveBeenCalledTimes(1);
		expect(cancelTurn.mock.calls[0]?.[0]).toMatchObject({
			id: "bt_1",
			bot_id: "bot_scout",
		});
		expect(panelText()).not.toContain("Working");
	});

	test("another bot's turn and a finished turn show nothing", async () => {
		await emitTurn({ id: "bt_2", bot_id: "bot_writer", status: "running" });
		expect(panelText()).not.toContain("Working");
		await emitTurn({ id: "bt_3", status: "running" });
		expect(panelText()).toContain("Working…");
		await emitTurn({ id: "bt_3", status: "done" });
		expect(panelText()).not.toContain("Working");
	});
});
