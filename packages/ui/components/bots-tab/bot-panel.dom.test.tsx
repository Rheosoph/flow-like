import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import type { IBot, IBotWorkspace } from "../../state/backend-state/bot-state";
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
	push: mock((_href: string) => {}),
};

const CHAT_BODY = "../global-chat/global-chat-body";
const EDITOR = "./editor";
const BOT_EXPRESSION = "../../hooks/use-bot-expression";
const VIEWS = "./workspace-views";
const MEMORY = "../../state/global-chat/global-chat-memory";
const FACTS = "../../state/global-chat/bot-memory-facts";

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
const savedFacts: Record<string, unknown[]> = {
	bot_scout: [
		{
			id: "f_digest",
			text: "Prefers the digest on Mondays",
			kind: "preference",
			subject: "user",
			source: "user_stated",
			confidence: 0.9,
			recorded_at: "2026-10-20T12:00:00Z",
			valid_from: null,
			valid_to: null,
			superseded_by: null,
			evidence: [
				{
					seq: 7,
					message_id: "m7",
					quote: "Send me the digest on Mondays",
					at: "x",
				},
			],
		},
		{
			id: "f_launch",
			text: "Works on the Q4 launch",
			kind: "profile",
			subject: "user",
			source: "user_stated",
			confidence: 0.8,
			recorded_at: "2026-10-21T12:00:00Z",
			valid_from: null,
			valid_to: null,
			superseded_by: null,
			evidence: [],
		},
	],
};
const facts = {
	list: mock(async (owner: { botId: string }) => [
		...(savedFacts[owner.botId] ?? []),
	]),
	forget: mock(async (_owner: { botId: string }, _id: string) => undefined),
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
	facts: { ...(await import(FACTS)) },
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
	useRouter: () => ({ replace: navigation.replace, push: navigation.push }),
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
mock.module(FACTS, () => ({
	...actual.facts,
	listBotFacts: facts.list,
	forgetBotFact: facts.forget,
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
const { ApiResponseError } = await import("../../lib/api-error");
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
	bot("bot_general", {
		system: "general",
		capabilities: ["internet", "memory"],
	}),
	bot("bot_flux", { system: "flux", capabilities: ["flowpilot"] }),
	bot("bot_scout", {
		name: "Scout",
		description: "Scout's job",
		capabilities: ["internet", "memory", "storage", "data_studio"],
	}),
	bot("bot_writer", {
		name: "Writer",
		description: "Writer's job",
		capabilities: ["memory"],
		embedding_model: "embed-a",
	}),
	bot("bot_pebble", {
		name: "Pebble",
		description: "Pebble's job",
		owner_id: "local",
		location: "desktop",
		local: true,
		capabilities: ["memory", "storage"],
	}),
	...["Ada", "Bea", "Cleo", "Dora"].map((name) =>
		bot(`bot_${name.toLowerCase()}`, {
			name,
			description: `${name}'s job`,
			capabilities: ["storage", "data_studio"],
		}),
	),
];
const scoutCopy = bot("bot_scout_copy", {
	name: "Scout copy",
	capabilities: ["internet", "memory", "storage", "data_studio"],
});

function workspaceOf(botId: string): Promise<IBotWorkspace> {
	return Promise.resolve({ app_id: `ws-${botId}` });
}

let respond = workspaceOf;
const getWorkspace = mock((botId: string) => respond(botId));
const cloneBot = mock(async (_botId: string) => scoutCopy);
const downloadStorageItems = mock(async (appId: string, prefixes: string[]) =>
	prefixes.map((prefix) => ({
		prefix,
		url: `https://files.test/${appId}/${prefix}`,
	})),
);

const botState: Record<string, unknown> = {
	listBots: async () => bots,
	getWorkspace,
	cloneBot,
};
const backend = {
	botState,
	appState: { getApps: async () => [] },
	inboxState: { listConversations: async () => [] },
	userState: {
		getProfile: async () => null,
		getInfo: async () => ({ id: "me", name: "Felix Schultz" }),
		lookupUser: async () => null,
		lookupUsers: async () => [],
		getSettingsProfile: async () => ({ hub_profile: { id: "profile-1" } }),
	},
	bitState: { getProfileBits: async () => [] },
	storageState: { downloadStorageItems },
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
	mock.module(FACTS, () => actual.facts);
	restoreDomGlobals();
});

function dock() {
	return container.querySelector("aside");
}

function pill() {
	return dock()?.querySelector<HTMLButtonElement>("[data-slot='bot-pill']");
}

function pillName() {
	return pill()?.querySelector("[data-slot='bot-pill-name']")?.textContent;
}

function panel() {
	return dock()?.querySelector("section[aria-label^='About']") ?? undefined;
}

function panelText() {
	return panel()?.textContent ?? "";
}

/** Checks as booleans, so a failure doesn't print a whole DOM tree. */
function panelOpen() {
	return panel() !== undefined;
}

function dockHas(selector: string) {
	return dock()?.querySelector(selector) != null;
}

function tabs() {
	return Array.from(panel()?.querySelectorAll("[role='tab']") ?? []).map(
		(tab) => tab.textContent,
	);
}

function activeTab() {
	return panel()?.querySelector("[role='tab'][data-state='active']")
		?.textContent;
}

/** The chat stays mounted under the sheet; `inert` keeps it out of reach while covered. */
function chatCovered() {
	const chat = dock()?.querySelector("[data-testid='global-chat']");
	expect(chat ?? undefined).toBeDefined();
	return chat?.closest("[inert]") !== null;
}

function lastUrl() {
	return navigation.replace.mock.calls.at(-1)?.[0];
}

function workspaceCalls(botId: string) {
	return getWorkspace.mock.calls.filter(([id]) => id === botId).length;
}

function buttonContaining(...texts: string[]) {
	return Array.from(container.querySelectorAll("button")).find((button) =>
		texts.every((text) => button.textContent?.includes(text)),
	);
}

function panelButton(label: string) {
	return Array.from(panel()?.querySelectorAll("button") ?? []).find(
		(button) =>
			button.textContent === label ||
			button.getAttribute("aria-label") === label,
	);
}

function panelLink(text: string) {
	return Array.from(
		panel()?.querySelectorAll<HTMLAnchorElement>("a[href]") ?? [],
	).find((link) => link.textContent?.includes(text));
}

async function click(element: { click(): void } | null | undefined) {
	expect(element ?? undefined).toBeDefined();
	await act(async () => {
		element?.click();
	});
	await settle();
}

/** Radix tabs switch on a primary-button mousedown. */
async function openTab(name: string) {
	const tab = Array.from(panel()?.querySelectorAll("[role='tab']") ?? []).find(
		(item) => item.textContent === name,
	);
	expect(tab).toBeDefined();
	await act(async () => {
		tab?.dispatchEvent(
			new window.MouseEvent("mousedown", { bubbles: true, button: 0 }) as never,
		);
	});
	await settle();
}

/** A reload or a link: the URL changes without the dock asking for it. */
async function visit(query: string) {
	navigation.params = new URLSearchParams(query);
	window.history.replaceState(null, "", query ? `/bots?${query}` : "/bots");
	await act(async () => root.render(view()));
	await settle();
}

describe("the pill and the bot's panel", () => {
	test("the chat is the main surface: a pill names the bot, with no header or tab strip", () => {
		expect(pillName()).toBe("Assistant");
		expect(pill()?.getAttribute("aria-expanded")).toBe("false");
		expect(pill()?.textContent).toContain("Ready");
		expect(dockHas("[role='tablist']")).toBe(false);
		expect(dockHas("h2")).toBe(false);
		expect(panelOpen()).toBe(false);
		expect(chatCovered()).toBe(false);
	});

	test("the pill opens the panel on Details; back returns to the chat", async () => {
		await click(pill());
		expect(lastUrl()).toBe("/bots?panel=details");
		expect(pill()?.getAttribute("aria-expanded")).toBe("true");
		expect(pill()?.getAttribute("aria-controls")).toBe(panel()?.id);
		expect(panel()?.querySelector("h2")?.textContent).toBe("Assistant");
		expect(tabs()).toEqual(["Details", "Memory"]);
		expect(activeTab()).toBe("Details");
		expect(chatCovered()).toBe(true);

		await click(panelButton("Back to the chat"));
		expect(lastUrl()).toBe("/bots");
		expect(panelOpen()).toBe(false);
		expect(pill()?.getAttribute("aria-expanded")).toBe("false");
		expect(chatCovered()).toBe(false);
	});

	test("its tabs follow the bot's abilities", async () => {
		await click(buttonContaining("Scout", "Runs in the cloud"));
		expect(pillName()).toBe("Scout");
		expect(pill()?.textContent).toContain("Runs in the cloud");
		await click(pill());
		expect(tabs()).toEqual(["Details", "Files", "Data", "Memory"]);
		expect(panelButton("Open workspace") === undefined).toBe(false);

		await click(buttonContaining("Writer", "Runs in the cloud"));
		expect(panelOpen()).toBe(false);
		await click(pill());
		expect(tabs()).toEqual(["Details", "Memory"]);
		expect(panelButton("Open workspace") === undefined).toBe(true);
		expect(getWorkspace).not.toHaveBeenCalled();
	});

	test("a bot on this device shows the drive in its pill and where it lives in Details", async () => {
		await click(buttonContaining("Pebble", "On this device"));
		expect(pillName()).toBe("Pebble");
		expect(pill()?.textContent).toContain("On this device");
		await click(pill());
		expect(tabs()).toEqual(["Details", "Files", "Memory"]);
		expect(panelText()).toContain("Where it runs");
		expect(panelText()).toContain(
			"Lives only on this computer until you move it to an account.",
		);
		expect(panelButton("Move to your account") === undefined).toBe(true);
	});

	test("Flux shows FlowPilot as always on and remembers per profile, without a workspace", async () => {
		await visit("bot=flux&panel=files");
		expect(pillName()).toBe("Flux");
		expect(tabs()).toEqual(["Details", "Memory"]);
		expect(activeTab()).toBe("Details");
		expect(panelText()).toContain("FlowPilot");
		expect(panelText()).toContain("Always on");
		expect(panelText()).not.toContain("Where it runs");
		expect(panelText()).not.toContain("Open workspace");
		expect(getWorkspace).not.toHaveBeenCalled();

		await openTab("Memory");
		const bookmark = new URL(lastUrl() ?? "", window.location.origin);
		expect(bookmark.searchParams.get("bot")).toBe("flux");
		expect(bookmark.searchParams.get("panel")).toBe("memory");
		expect(bookmark.searchParams.get("session")).toBe(
			useGlobalChatStore.getState().activeConversationId,
		);
		expect(memory.list).toHaveBeenLastCalledWith(
			{ profileId: "profile-1", botId: null },
			"token",
		);
	});
});

describe("Details", () => {
	test("says where it runs, what it may do and the models it uses", async () => {
		await visit("bot=bot_scout&panel=details");
		const text = panelText();
		expect(text).toContain("Scout's job");
		expect(text).toContain("Cloud");
		expect(text).toContain("Runs on Flow-Like's servers, around the clock");
		expect(text).toContain("Automatic");
		expect(text).toContain("Let the host choose");
		for (const ability of ["Internet", "Memory", "Files", "Data Studio"])
			expect(text).toContain(ability);
	});

	test("Edit opens the bot's settings", async () => {
		await click(panelButton("Edit"));
		expect(editor).toEqual({ open: true, botId: "bot_scout" });
	});

	test("Duplicate opens the copy", async () => {
		await click(panelButton("Duplicate"));
		await settle();
		expect(cloneBot).toHaveBeenLastCalledWith("bot_scout");
		expect(lastUrl()).toBe("/bots?bot=bot_scout_copy");
		expect(pillName()).toBe("Scout copy");
	});
});

describe("the bot's workspace in the panel", () => {
	test("Files sets up the workspace first, then browses it, and ?panel= remembers the tab", async () => {
		let release: (() => void) | undefined;
		respond = (botId) =>
			botId === "bot_scout"
				? new Promise((resolve) => {
						release = () => resolve({ app_id: "ws-scout" });
					})
				: workspaceOf(botId);
		await click(buttonContaining("Scout", "Runs in the cloud"));
		expect(lastUrl()).toBe("/bots?bot=bot_scout");
		await click(pill());
		await openTab("Files");
		expect(lastUrl()).toBe("/bots?bot=bot_scout&panel=files");
		expect(activeTab()).toBe("Files");
		expect(panelText()).toContain("Setting up Scout's workspace…");
		expect(workspaceCalls("bot_scout")).toBe(1);

		await act(async () => release?.());
		await settle();
		expect(panelText()).toContain("Files of ws-scout");
		expect(panelLink("Open full view")?.getAttribute("href")).toBe(
			"/library/config/storage?id=ws-scout",
		);
		expect(panelLink("Open workspace")?.getAttribute("href")).toBe(
			"/library/config?id=ws-scout",
		);
		const files = shown.storage.at(-1);
		expect(files?.appId).toBe("ws-scout");
		expect(files?.prefix).toBe("");
		expect(files?.title).toBe("Scout's files");
		expect(files?.canWrite).toBe(true);
		expect(await files?.fileToUrl("upload/notes.md")).toBe(
			"https://files.test/ws-scout/upload/notes.md",
		);
		expect(downloadStorageItems).toHaveBeenLastCalledWith("ws-scout", [
			"upload/notes.md",
		]);
		respond = workspaceOf;
	});

	test("Data opens the same workspace without asking again, with the chat kept under it", async () => {
		await openTab("Data");
		expect(lastUrl()).toBe("/bots?bot=bot_scout&panel=data");
		expect(panelText()).toContain("Tables of ws-scout");
		expect(panelLink("Open full view")?.getAttribute("href")).toBe(
			"/library/config/explore?id=ws-scout",
		);
		expect(shown.studio.at(-1)).toBe("ws-scout");
		expect(chatCovered()).toBe(true);

		await openTab("Files");
		await click(panelButton("Back to the chat"));
		expect(lastUrl()).toBe("/bots?bot=bot_scout");
		expect(chatCovered()).toBe(false);
		expect(workspaceCalls("bot_scout")).toBe(1);
	});

	test("the URL brings the panel back; another bot opens with it closed and without Data Studio's place", async () => {
		await visit("bot=bot_scout&panel=data&table=facts&view=queries");
		expect(activeTab()).toBe("Data");
		expect(panelText()).toContain("Tables of ws-scout");

		await click(buttonContaining("Writer", "Runs in the cloud"));
		expect(lastUrl()).toBe("/bots?bot=bot_writer");
		expect(pillName()).toBe("Writer");
		expect(panelOpen()).toBe(false);
		expect(workspaceCalls("bot_writer")).toBe(0);
	});

	test("the header link sets the workspace up only when followed, then links straight to it", async () => {
		await visit("bot=bot_pebble&panel=details");
		expect(workspaceCalls("bot_pebble")).toBe(0);
		expect(panelLink("Open workspace") === undefined).toBe(true);

		await click(panelButton("Open workspace"));
		expect(workspaceCalls("bot_pebble")).toBe(1);
		expect(navigation.push).toHaveBeenLastCalledWith(
			"/library/config?id=ws-bot_pebble",
		);
		expect(panelLink("Open workspace")?.getAttribute("href")).toBe(
			"/library/config?id=ws-bot_pebble",
		);
	});

	test("Memory lists the bot's facts with their quote and forgets one", async () => {
		await visit("bot=bot_scout&panel=memory");
		expect(facts.list).toHaveBeenLastCalledWith(
			expect.objectContaining({ botId: "bot_scout", appId: "ws-scout" }),
		);
		expect(panelText()).toContain("Prefers the digest on Mondays");
		expect(panelText()).toContain("Works on the Q4 launch");
		expect(panelText()).toContain("Send me the digest on Mondays");
		expect(panelText()).toContain("#7");

		const forget = panel()?.querySelectorAll<HTMLButtonElement>(
			"button[aria-label='Forget this fact']",
		);
		expect(forget?.length).toBe(2);
		await click(forget?.[0]);
		expect(facts.forget).toHaveBeenLastCalledWith(
			expect.objectContaining({ botId: "bot_scout" }),
			"f_digest",
		);
		expect(panelText()).not.toContain("Prefers the digest on Mondays");
	});

	test("a bot without facts says how they come about", async () => {
		await visit("bot=bot_writer&panel=memory");
		expect(panelText()).toContain(
			"Writer writes down facts, preferences and decisions here as you chat.",
		);
	});

	test("the chat comes back when it needs to be seen", async () => {
		expect(chatCovered()).toBe(true);
		await act(async () => useGlobalChatStore.getState().requestReveal());
		await settle();
		expect(panelOpen()).toBe(false);
		expect(chatCovered()).toBe(false);
		expect(lastUrl()).toBe("/bots?bot=bot_writer");
	});

	test("a refusal shows the hub's words and a hub without workspaces says so", async () => {
		respond = (botId) => {
			if (botId === "bot_ada") {
				return Promise.reject(
					new ApiResponseError({
						status: 409,
						code: "BOT_WORKSPACE_REFUSED",
						message: "This bot can't have a workspace yet.",
					}),
				);
			}
			if (botId === "bot_cleo") {
				return Promise.reject(
					new ApiResponseError({ status: 404, message: "Not Found" }),
				);
			}
			return workspaceOf(botId);
		};
		await visit("bot=bot_ada&panel=files");
		expect(panelText()).toContain("Ada can't keep its own files and data yet.");
		expect(panelText()).toContain("This bot can't have a workspace yet.");
		expect(dockHas("[data-testid='storage']")).toBe(false);

		await visit("bot=bot_cleo&panel=data");
		expect(panelText()).toContain(
			"Files and data for Cleo aren't available here yet.",
		);
		expect(dockHas("[data-testid='data-studio']")).toBe(false);
		respond = workspaceOf;
	});

	test("a failed setup says why and can be tried again", async () => {
		let attempts = 0;
		respond = (botId) => {
			if (botId === "bot_bea" && attempts++ === 0) {
				return Promise.reject(new Error("The hub took too long to answer"));
			}
			return workspaceOf(botId);
		};
		await visit("bot=bot_bea&panel=files");
		expect(panelText()).toContain("Couldn't set up Bea's workspace.");
		expect(panelText()).toContain("The hub took too long to answer");

		await click(panelButton("Try again"));
		expect(workspaceCalls("bot_bea")).toBe(2);
		expect(panelText()).toContain("Files of ws-bot_bea");
		respond = workspaceOf;
	});

	test("a host that can't make workspaces says so instead of asking", async () => {
		const { getWorkspace: hostCall } = botState;
		Reflect.deleteProperty(botState, "getWorkspace");
		try {
			await visit("bot=bot_dora&panel=files");
			expect(activeTab()).toBe("Files");
			expect(panelText()).toContain(
				"Files and data for Dora aren't available here yet.",
			);
			expect(workspaceCalls("bot_dora")).toBe(0);
		} finally {
			botState.getWorkspace = hostCall;
		}
	});

	test("Show chat reopens a closed dock on the chat, not under the sheet", async () => {
		expect(chatCovered()).toBe(true);
		await click(panelButton("Close chat"));
		expect(dock() === null).toBe(true);

		await click(buttonContaining("Show chat"));
		expect(pillName()).toBe("Dora");
		expect(panelOpen()).toBe(false);
		expect(chatCovered()).toBe(false);
		expect(lastUrl()).toBe("/bots?bot=bot_dora");
	});
});
