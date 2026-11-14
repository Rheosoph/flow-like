import { afterAll, beforeAll, describe, expect, mock, test } from "bun:test";
import { act, createElement } from "react";
import type {
	IBot,
	IBotMoveIssue,
	IBotMoveProgress,
	IBotMoveReport,
	IMovedBot,
	IUpdateBotRequest,
} from "../../state/backend-state/bot-state";
import type { IConversation } from "../../state/backend-state/inbox-state";
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

interface EditorProps {
	open: boolean;
	botId?: string | null;
	onOpenChange: (open: boolean) => void;
}
const editor: { props?: EditorProps } = {};
const MODELS = [{ id: "model-a", label: "Model A", local: false, free: false }];

const CHAT_BODY = "../global-chat/global-chat-body";
const EDITOR = "./editor";
const MODEL_OPTIONS = "./editor/steps/use-bot-models";
const BOT_EXPRESSION = "../../hooks/use-bot-expression";

const actual = {
	locales: { ...(await import("@flow-like/locales")) },
	navigation: { ...(await import("next/navigation")) },
	oidc: { ...(await import("react-oidc-context")) },
	chatBody: { ...(await import(CHAT_BODY)) },
	editor: { ...(await import(EDITOR)) },
	models: { ...(await import(MODEL_OPTIONS)) },
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
			profile: { sub: "me", given_name: "Felix", email: "felix@example.com" },
		},
	}),
}));
mock.module(CHAT_BODY, () => ({
	...actual.chatBody,
	GlobalChatBody: () =>
		createElement("div", { "data-testid": "global-chat" }, "Global chat"),
}));
mock.module(EDITOR, () => ({
	...actual.editor,
	BotEditorDialog: (props: EditorProps) => {
		editor.props = props;
		return null;
	},
}));
mock.module(MODEL_OPTIONS, () => ({
	...actual.models,
	useBotModelOptions: () => ({ options: MODELS, isLoading: false }),
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
const { BotsView } = await import("./bots-view");

const SCOUT = "bot_scoutlocal000000001";
const WRITER = "bot_writerlocal00000001";
const PAINTER = "bot_painterlocal0000001";
const HELPER = "bot_helperlocal00000001";
const NAMES: Record<string, string> = {
	[SCOUT]: "Scout",
	[WRITER]: "Writer",
	[PAINTER]: "Painter",
	[HELPER]: "Helper",
};

function bot(id: string, overrides: Partial<IBot> = {}): IBot {
	return {
		id,
		owner_id: "local",
		system: null,
		preset_id: null,
		name: NAMES[id] ?? null,
		description: null,
		instructions: null,
		appearance: { kind: "mochi" },
		model: { primary: "model-a", fallbacks: [] },
		classification: { pii: false },
		capabilities: ["internet"],
		profile_id: null,
		location: "desktop",
		device_id: null,
		created_at: "",
		updated_at: "",
		local: true,
		...overrides,
	};
}

const builtIn = [
	bot("bot_general", { system: "general", owner_id: "me", local: undefined }),
	bot("bot_flux", { system: "flux", owner_id: "me", local: undefined }),
];

const bots = new Map<string, IBot>([
	[
		SCOUT,
		bot(SCOUT, {
			capabilities: ["internet", "profile_apps"],
			profile_id: "profile-old",
			model: { primary: "model-x", fallbacks: ["model-a"] },
		}),
	],
	[WRITER, bot(WRITER)],
	[PAINTER, bot(PAINTER, { appearance: { kind: "retired-character" } })],
	[
		HELPER,
		bot(HELPER, { model: { primary: "model-a", fallbacks: ["model-gone"] } }),
	],
]);
const attachedApps = new Map<string, string[]>([[SCOUT, ["app_atlas"]]]);
/** Secrets this device's key no longer opens. */
const unreadableSecrets = new Map<string, string[]>([[SCOUT, ["OLD_TOKEN"]]]);
/** Bots the server would refuse because the plan's bots on the server are used up. */
const serverFull = new Set<string>();

/** What the server's import check and this computer's store say about a local bot right now. */
function report(botId: string): IBotMoveReport {
	const current = bots.get(botId) as IBot;
	const apps = attachedApps.get(botId) ?? [];
	const issues: IBotMoveIssue[] = apps.map((appId) => ({
		part: "app",
		code: "BOT_APP_UNAVAILABLE",
		message: "The app isn't available",
		ref_id: appId,
	}));
	if (
		current.capabilities.includes("profile_apps") &&
		current.profile_id !== "profile-now"
	) {
		issues.push({
			part: "profile",
			code: "BOT_PROFILE_UNAVAILABLE",
			message: "Profile not found",
			ref_id: current.profile_id,
		});
	}
	for (const name of unreadableSecrets.get(botId) ?? []) {
		issues.push({
			part: "secret",
			code: "BOT_SECRET_UNREADABLE",
			message: `Secret ${name} can't be read on this computer anymore`,
			ref_id: name,
		});
	}
	if (serverFull.has(botId)) {
		issues.push({
			part: "bot",
			code: "BOT_SERVER_LIMIT_REACHED",
			message:
				"Your FREE plan allows 3 bots on the server and you have 3; move one to a desktop or device, delete one, or upgrade your plan",
			ref_id: null,
		});
	}
	if (botId === PAINTER) {
		issues.push({
			part: "bot",
			code: "BOT_LIMIT_REACHED",
			message: "Bot limit reached",
			ref_id: null,
		});
	}
	const scout = botId === SCOUT;
	return {
		bot_id: botId,
		issues,
		apps: apps.length,
		secrets: scout ? 2 : 1,
		conversations: scout ? 3 : 1,
		messages: scout ? 41 : 1,
		waiting_conversations: scout ? 1 : 0,
		skipped_messages: scout ? 2 : 0,
	};
}

/** The account gives a moved bot its own id. */
const accountIdOf = (botId: string) => botId.replace("local", "acct");

/** A chat from before signing in: you are still `local` in it. */
const painterChat: IConversation = {
	id: "local-painter",
	kind: "direct",
	title: null,
	encryption: "plain",
	created_by: "local",
	created_at: "",
	last_activity_at: "",
	members: [
		{ principal_id: "local", kind: "user", added_by: null, joined_at: "" },
		{ principal_id: PAINTER, kind: "bot", added_by: "local", joined_at: "" },
	],
	unread_count: 0,
	last_entry: {
		id: "e1",
		message_id: "m1",
		conversation_id: "local-painter",
		sender_id: "local",
		kind: "message",
		envelope: { v: 1, enc: "none", body: { text: "Paint the hallway" } },
		created_at: "",
		cursor: "1:e1",
	},
	local: true,
};

interface PendingMove {
	botId: string;
	resolve: (warnings?: number) => void;
	reject: (error: Error) => void;
}
const moves: PendingMove[] = [];

const progressListeners = new Set<(progress: IBotMoveProgress) => void>();
const emitProgress = (progress: IBotMoveProgress) =>
	act(async () => {
		for (const listener of progressListeners) listener(progress);
	});

const localHost = {
	checkMove: mock(async (botId: string) => report(botId)),
	onProgress: mock(async (listener: (progress: IBotMoveProgress) => void) => {
		progressListeners.add(listener);
		return () => progressListeners.delete(listener);
	}),
	move: mock(
		(botId: string) =>
			new Promise<IMovedBot>((resolve, reject) => {
				moves.push({
					botId,
					resolve: (warnings = 0) => {
						const moved: IBot = {
							...(bots.get(botId) as IBot),
							id: accountIdOf(botId),
							owner_id: "me",
							local: false,
						};
						bots.delete(botId);
						bots.set(moved.id, moved);
						resolve({ bot: moved, warnings });
					},
					reject,
				});
			}),
	),
};

const rebindBotChat = mock((_fromBotId: string, _toBotId: string) => {});
const previousRebind = (
	useGlobalChatStore.getState() as { rebindBotChat?: unknown }
).rebindBotChat;

const updateBot = mock(async (botId: string, request: IUpdateBotRequest) => {
	const next = { ...(bots.get(botId) as IBot), ...request } as IBot;
	bots.set(botId, next);
	return next;
});
const detachBotApp = mock(async (botId: string, appId: string) => {
	attachedApps.set(
		botId,
		(attachedApps.get(botId) ?? []).filter((id) => id !== appId),
	);
});
const deleteBotSecret = mock(async (botId: string, name: string) => {
	unreadableSecrets.set(
		botId,
		(unreadableSecrets.get(botId) ?? []).filter((each) => each !== name),
	);
});

const backend = {
	botState: {
		local: localHost,
		listBots: async () => [...builtIn, ...bots.values()],
		getBot: async (botId: string) => bots.get(botId),
		updateBot,
		detachBotApp,
		deleteBotSecret,
		attachBotApp: async () => {},
		listBotApps: async () => [],
	},
	inboxState: { listConversations: async () => [painterChat] },
	userState: {
		getInfo: async () => ({ id: "me", name: "Felix Schultz" }),
		getSettingsProfile: async () => ({ hub_profile: { id: "profile-now" } }),
		getProfile: async () => null,
		lookupUser: async () => null,
		lookupUsers: async () => [],
	},
	appState: {
		getApps: async () => [[{ id: "app_atlas" }, { name: "Atlas" }]],
	},
};

const previousBackend = useBackendStore.getState().backend;
const previousSignedIn = useAuthStatusStore.getState().signedIn;
const client = new QueryClient({
	defaultOptions: { queries: { retry: false } },
});
let root: ReturnType<typeof createRoot>;
let container: HTMLElement;
const view = () =>
	createElement(QueryClientProvider, { client }, createElement(BotsView));

function dialog() {
	return window.document.body.querySelector(
		"[role='dialog']",
	) as HTMLElement | null;
}

/** The review's row for one bot. */
function row(botId: string) {
	return Array.from(dialog()?.querySelectorAll("li") ?? []).find(
		(item) => item.querySelector("span.truncate")?.textContent === NAMES[botId],
	) as HTMLElement | undefined;
}

/** Booleans, so a failure never pretty-prints a whole DOM node. */
const reviewOpen = () => dialog() !== null;
const listed = (botId: string) => row(botId) !== undefined;

function buttonIn(scope: ParentNode | null | undefined, label: string) {
	return Array.from(scope?.querySelectorAll("button") ?? []).find((button) =>
		button.textContent?.includes(label),
	) as HTMLButtonElement | undefined;
}

function buttonLabels(scope: ParentNode | null | undefined) {
	return Array.from(scope?.querySelectorAll("button") ?? []).map(
		(button) => button.textContent,
	);
}

async function click(element: { click(): void } | null | undefined) {
	expect(element ?? undefined).toBeDefined();
	await act(async () => {
		element?.click();
	});
	await settle();
}

function checksOf(botId: string) {
	return localHost.checkMove.mock.calls.filter(([id]) => id === botId).length;
}

beforeAll(async () => {
	installDomGlobals();
	useAuthStatusStore.getState().setSignedIn(true);
	useBackendStore.getState().setBackend(backend as never);
	useGlobalChatStore.setState({ rebindBotChat } as never);
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
	useGlobalChatStore.setState({ rebindBotChat: previousRebind } as never);
	window.history.replaceState(null, "", "/bots");
	await new Promise((resolve) => setTimeout(resolve, 0));
	mock.restore();
	mock.module("@flow-like/locales", () => actual.locales);
	mock.module("next/navigation", () => actual.navigation);
	mock.module("react-oidc-context", () => actual.oidc);
	mock.module(CHAT_BODY, () => actual.chatBody);
	mock.module(EDITOR, () => actual.editor);
	mock.module(MODEL_OPTIONS, () => actual.models);
	mock.module(BOT_EXPRESSION, () => actual.expression);
	restoreDomGlobals();
});

describe("moving bots from this computer to the account", () => {
	test("signed in, a chat from before reads as yours next to the account's", () => {
		const text = container.textContent ?? "";
		expect(text).toContain("You: Paint the hallway");
		expect(
			container.querySelector(
				"[aria-label='This chat lives only on this computer.']",
			) !== null,
		).toBe(true);
		expect(
			container.querySelector("[aria-label='Painter, Your bot']") !== null,
		).toBe(true);
	});

	test("the banner opens a review of every bot on this computer", async () => {
		const text = container.textContent ?? "";
		expect(text).toContain("4 bots live only on this computer");
		expect(text.split("Local only").length - 1).toBe(4);

		await click(buttonIn(container, "Review and move"));
		const review = dialog();
		expect(review?.textContent).toContain("Move bots to your account");
		expect(review?.textContent).toContain("Signed in as felix@example.com");
		for (const botId of [SCOUT, WRITER, PAINTER, HELPER]) {
			expect(listed(botId)).toBe(true);
		}
		const scout = row(SCOUT)?.textContent ?? "";
		expect(scout).toContain("1 app");
		expect(scout).toContain("2 secrets, stored encrypted in your account");
		expect(scout).toContain("3 chats with 41 messages");
		expect(scout).toContain("1 chat moves once the other bots in it do");
		expect(scout).toContain(
			"2 messages from bots that left these chats are left out",
		);
		expect(scout).toContain("4 issues");
		expect(row(WRITER)?.textContent).toContain("Ready");
		expect(buttonIn(review, "Move 1 bot")?.disabled).toBe(false);
	});

	test("lists the server's issues next to the client's model and look checks", () => {
		const scout = row(SCOUT)?.textContent ?? "";
		expect(scout).toContain("Atlas isn't available to your account.");
		expect(scout).toContain(
			"The profile its apps come from isn't in your account.",
		);
		expect(scout).toContain(
			"Its model model-x isn't available to your account.",
		);
		expect(scout).toContain(
			"Its secret OLD_TOKEN can't be read on this computer anymore. Remove it here and add it again once the bot has moved.",
		);
		expect(row(HELPER)?.textContent).toContain(
			"Its fallback model model-gone isn't available to your account.",
		);

		const painter = row(PAINTER);
		expect(painter?.textContent).toContain(
			"Your account already holds as many bots as it can. It stays on this computer and keeps working.",
		);
		expect(painter?.textContent).toContain(
			"Its look uses items your account doesn't own:",
		);
		expect(buttonLabels(painter)).toEqual(["Open settings"]);
	});

	test("each fix changes the bot through its state and checks it again", async () => {
		let checks = checksOf(SCOUT);
		await click(buttonIn(row(SCOUT), "Remove from bot"));
		expect(detachBotApp).toHaveBeenCalledWith(SCOUT, "app_atlas");
		expect(checksOf(SCOUT)).toBe(checks + 1);
		expect(row(SCOUT)?.textContent).not.toContain("Atlas isn't available");

		checks = checksOf(SCOUT);
		await click(buttonIn(row(SCOUT), "Use current profile"));
		expect(updateBot).toHaveBeenLastCalledWith(SCOUT, {
			profile_id: "profile-now",
		});
		expect(checksOf(SCOUT)).toBe(checks + 1);
		expect(row(SCOUT)?.textContent).not.toContain("The profile its apps");

		checks = checksOf(SCOUT);
		await click(buttonIn(row(SCOUT), "Remove secret"));
		expect(deleteBotSecret).toHaveBeenCalledWith(SCOUT, "OLD_TOKEN");
		expect(checksOf(SCOUT)).toBe(checks + 1);
		expect(row(SCOUT)?.textContent).not.toContain("Its secret OLD_TOKEN");

		checks = checksOf(SCOUT);
		await click(buttonIn(row(SCOUT), "Let the host choose"));
		expect(updateBot).toHaveBeenLastCalledWith(SCOUT, {
			model: { primary: null, fallbacks: ["model-a"] },
		});
		expect(checksOf(SCOUT)).toBe(checks + 1);
		expect(row(SCOUT)?.textContent).toContain("Ready");

		checks = checksOf(HELPER);
		await click(buttonIn(row(HELPER), "Remove fallback"));
		expect(updateBot).toHaveBeenLastCalledWith(HELPER, {
			model: { primary: "model-a", fallbacks: [] },
		});
		expect(checksOf(HELPER)).toBe(checks + 1);
		expect(buttonIn(dialog(), "Move 3 bots")?.disabled).toBe(false);
	});

	test("Open settings edits the bot and its review is checked again afterwards", async () => {
		await click(buttonIn(row(PAINTER), "Open settings"));
		expect(editor.props).toMatchObject({ open: true, botId: PAINTER });

		const checks = checksOf(WRITER);
		await act(async () => editor.props?.onOpenChange(false));
		await settle();
		expect(checksOf(WRITER)).toBe(checks + 1);
		expect(reviewOpen()).toBe(true);
	});

	test("moves the ready bots one at a time and keeps failed ones here", async () => {
		await click(buttonIn(dialog(), "Move 3 bots"));
		expect(moves.map((move) => move.botId)).toEqual([SCOUT]);
		expect(dialog()?.querySelector("output")?.textContent).toBe(
			"Moving 1 of 3…",
		);
		expect(row(SCOUT)?.textContent).toContain("Moving…");

		await emitProgress({ bot_id: SCOUT, step: "workspace", pass: 1 });
		expect(row(SCOUT)?.textContent).toContain("Copying its files…");
		await emitProgress({ bot_id: SCOUT, step: "segments", pass: 1 });
		expect(row(SCOUT)?.textContent).toContain("Moving its chat summaries…");
		expect(row(SCOUT)?.textContent).not.toContain("Copying its files…");

		await act(async () => moves[0]?.resolve(3));
		await settle();
		expect(row(SCOUT)?.textContent).toContain("3 file links couldn't move");
		expect(moves.map((move) => move.botId)).toEqual([SCOUT, WRITER]);
		expect(dialog()?.querySelector("output")?.textContent).toBe(
			"Moving 2 of 3…",
		);

		const helperChecks = checksOf(HELPER);
		await act(async () =>
			moves[1]?.reject(new Error("The hub is unreachable")),
		);
		await settle();
		expect(moves.map((move) => move.botId)).toEqual([SCOUT, WRITER, HELPER]);
		await act(async () =>
			moves[2]?.reject(
				Object.assign(
					new Error(`Could not move bot ${HELPER}: [BOT_LIMIT_REACHED]`),
					{ code: "BOT_LIMIT_REACHED" },
				),
			),
		);
		await settle();
		await settle();

		expect(row(SCOUT)?.textContent).toContain("Moved");
		expect(row(WRITER)?.textContent).not.toContain("couldn't move");
		expect(row(WRITER)?.textContent).toContain(
			"Not moved, so it stays on this computer. The hub is unreachable",
		);
		expect(row(HELPER)?.textContent).toContain(
			"Not moved, so it stays on this computer. Your account already holds as many bots as it can.",
		);
		expect(checksOf(HELPER)).toBe(helperChecks + 1);
		expect(row(HELPER)?.textContent).toContain("Ready");
		expect(dialog()?.querySelector("output")?.textContent).toBe("");
		expect(rebindBotChat.mock.calls).toEqual([[SCOUT, accountIdOf(SCOUT)]]);
		expect(navigation.replace).toHaveBeenLastCalledWith(
			`/bots?bot=${accountIdOf(SCOUT)}`,
			{ scroll: false },
		);
		expect(container.textContent).toContain(
			"3 bots live only on this computer",
		);
	});

	test("a move that kept meeting changes here says so, and the bot stays", async () => {
		const before = moves.length;
		await click(buttonIn(dialog(), "Move 2 bots"));
		expect(moves.slice(before).map((move) => move.botId)).toEqual([WRITER]);
		await act(async () =>
			moves[before]?.reject(
				Object.assign(
					new Error(
						`Could not finish moving bot ${WRITER}: it kept changing on this computer while it moved.`,
					),
					{ code: "LOCAL_CHANGED" },
				),
			),
		);
		await settle();
		await act(async () =>
			moves[before + 1]?.reject(new Error("The hub is unreachable")),
		);
		await settle();
		await settle();

		expect(row(WRITER)?.textContent).toContain(
			"Not moved, so it stays on this computer. It kept changing on this computer while it moved. Try again in a moment.",
		);
		expect(rebindBotChat.mock.calls).toHaveLength(1);
		expect(container.textContent).toContain(
			"3 bots live only on this computer",
		);
	});

	test("closing the review leaves the URL alone when it wasn't asked for there", async () => {
		const writes = navigation.replace.mock.calls.length;
		await click(buttonIn(dialog(), "Close"));
		expect(reviewOpen()).toBe(false);
		expect(navigation.replace.mock.calls.length).toBe(writes);
	});

	test("`?move=local` opens the review and closing it drops only that parameter", async () => {
		const href = `/bots?move=local&bot=${HELPER}`;
		navigation.params = new URLSearchParams(href.split("?")[1]);
		window.history.replaceState(null, "", href);
		await act(async () => root.render(view()));
		await settle();
		expect(dialog()?.textContent).toContain("Move bots to your account");
		expect(listed(PAINTER)).toBe(true);
		expect(listed(HELPER)).toBe(true);

		await click(buttonIn(dialog(), "Close"));
		expect(navigation.replace).toHaveBeenLastCalledWith(`/bots?bot=${HELPER}`, {
			scroll: false,
		});
		expect(reviewOpen()).toBe(false);
	});

	test("a local bot's panel moves just that bot", async () => {
		await click(buttonIn(container, "Painter"));
		const pill = container.querySelector<HTMLButtonElement>(
			"aside [data-slot='bot-pill']",
		);
		expect(pill?.textContent).toContain("Local only");
		expect(
			container.querySelector("aside [data-testid='global-chat']"),
		).not.toBeNull();
		await click(pill);
		expect(
			container.querySelector("aside section[aria-label^='About']")
				?.textContent,
		).toContain(
			"Move it to your account to connect it with your other bots. It will still run in the desktop app.",
		);
		await click(
			buttonIn(
				container.querySelector("aside section[aria-label^='About']"),
				"Move to your account",
			),
		);
		expect(listed(PAINTER)).toBe(true);
		expect(listed(HELPER)).toBe(false);
		await click(buttonIn(dialog(), "Close"));
	});

	test("a full plan on the server keeps the bot here, with the plan's numbers and the ways out", async () => {
		serverFull.add(HELPER);
		const href = `/bots?move=local&bot=${HELPER}`;
		navigation.params = new URLSearchParams(href.split("?")[1]);
		window.history.replaceState(null, "", href);
		await act(async () => root.render(view()));
		await settle();

		const line = Array.from(row(HELPER)?.querySelectorAll("li") ?? []).find(
			(item) => item.textContent?.includes("Your FREE plan"),
		);
		expect(line?.textContent).toBe(
			"Your FREE plan allows 3 bots on the server and you have 3. Delete one of your bots on the server or upgrade your plan to move it. It stays on this computer and keeps working.",
		);
		expect(buttonLabels(line)).toEqual([]);

		serverFull.delete(HELPER);
		await click(buttonIn(dialog(), "Close"));
	});
});
