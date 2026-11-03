// @vitest-environment happy-dom
import "fake-indexeddb/auto";
import botsLocale from "@flow-like/locales/locales/en/bots.json";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { useAuthStatusStore, useBackendStore } from "../../state/backend-state";
import type { IBot } from "../../state/backend-state/bot-state";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { GlobalChatOverlay } from "./global-chat-overlay";

const controls = vi.hoisted(() => ({
	push: vi.fn(),
	editor: {} as {
		open?: boolean;
		botId?: string | null;
		onOpenChange?: (open: boolean) => void;
	},
}));

function english(key: string): string {
	const value = key
		.split(".")
		.reduce<unknown>(
			(node, part) => (node as Record<string, unknown> | undefined)?.[part],
			botsLocale,
		);
	return typeof value === "string" ? value : key;
}

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (key: string, fallback?: unknown, options?: Record<string, unknown>) => {
			const values = typeof fallback === "object" ? fallback : options;
			const template = typeof fallback === "string" ? fallback : english(key);
			return template.replace(/\{\{(\w+)\}\}/g, (_, name: string) =>
				String((values as Record<string, unknown> | undefined)?.[name] ?? ""),
			);
		},
		i18n: { language: "en" },
	}),
}));
vi.mock("next/navigation", () => ({
	useRouter: () => ({ push: controls.push }),
	usePathname: () => "/library",
}));
vi.mock("./global-chat-body", () => ({
	GlobalChatBody: () => <div data-testid="chat-body" />,
}));
vi.mock("../bots-tab/editor/bot-editor-dialog", () => ({
	BotEditorDialog: (props: typeof controls.editor) => {
		controls.editor = props;
		return null;
	},
}));
vi.mock("../../hooks/use-bot-expression", () => ({
	useBotExpression: () => ({ expression: "neutral", status: "idle" }),
}));
vi.mock("../bots/bot-avatar", () => ({
	BotAvatar: ({ appearance }: { appearance?: { kind?: string } }) => (
		<span data-kind={appearance?.kind ?? "default"} />
	),
}));
// happy-dom cancels Web Animations with an unhandled AbortError.
vi.mock("framer-motion", () => ({
	AnimatePresence: ({ children }: { children: ReactNode }) => children,
	motion: {
		div: ({
			initial: _initial,
			animate: _animate,
			exit: _exit,
			transition: _transition,
			...props
		}: Record<string, unknown>) => <div {...props} />,
	},
}));

function bot(id: string, overrides: Partial<IBot> = {}): IBot {
	return {
		id,
		owner_id: "me",
		system: null,
		preset_id: null,
		name: "Scout",
		description: "Research digest",
		instructions: null,
		appearance: { kind: "mochi" } as IBot["appearance"],
		model: { primary: null, fallbacks: [] },
		classification: {} as IBot["classification"],
		capabilities: ["internet"],
		profile_id: null,
		location: "server",
		device_id: null,
		created_at: "",
		updated_at: "",
		...overrides,
	};
}

const scout = bot("bot_scout");
const pebble = bot("bot_pebble", {
	name: "Pebble",
	owner_id: "local",
	location: "desktop",
	local: true,
});
const cloneBot = vi.fn(async () =>
	bot("bot_scout_copy", { name: "Scout copy" }),
);

let root: Root;
let host: HTMLDivElement;

async function settle() {
	await act(async () => new Promise((resolve) => setTimeout(resolve, 20)));
}

beforeEach(async () => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	useAuthStatusStore.setState({ signedIn: true });
	useBackendStore.setState({
		backend: {
			botState: {
				listBots: async () => [scout, pebble],
				cloneBot,
				local: { list: async () => [] },
			},
		} as never,
	});
	useGlobalChatStore.setState({
		mode: "overlay",
		activeConversationId: "flux-1",
		messages: [],
		toolPrompt: null,
		runs: {},
	});
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<GlobalChatOverlay />
			</QueryClientProvider>,
		),
	);
	await settle();
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	useGlobalChatStore.setState({ mode: "closed", toolPrompt: null });
	useBackendStore.setState({ backend: null });
	controls.push.mockClear();
	controls.editor = {};
	cloneBot.mockClear();
});

function pill() {
	return document.body.querySelector<HTMLButtonElement>(
		"[data-slot='bot-pill']",
	);
}

function pillShows() {
	return {
		name: pill()?.querySelector("[data-slot='bot-pill-name']")?.textContent,
		look: pill()?.querySelector("[data-kind]")?.getAttribute("data-kind"),
	};
}

function panel() {
	return document.body.querySelector("section[aria-label^='About']");
}

function panelButton(label: string) {
	return Array.from(panel()?.querySelectorAll("button") ?? []).find(
		(button) =>
			button.textContent === label ||
			button.getAttribute("aria-label") === label,
	);
}

async function show(conversationId: string) {
	await act(async () =>
		useGlobalChatStore.setState({ activeConversationId: conversationId }),
	);
}

async function click(element: HTMLElement | null | undefined) {
	expect(element ?? undefined).toBeDefined();
	await act(async () => element?.click());
	await settle();
}

/** Opening the full chat closes the overlay, so each call opens it again first. */
async function openFullChat() {
	await act(async () => useGlobalChatStore.setState({ mode: "overlay" }));
	const button = document.body.querySelector<HTMLButtonElement>(
		"button[aria-label='Open full chat']",
	);
	expect(button).not.toBeNull();
	await act(async () => button?.click());
	expect(useGlobalChatStore.getState().mode).toBe("closed");
	return controls.push.mock.lastCall?.[0];
}

test("the pill shows the bot the open conversation belongs to", async () => {
	expect(document.body.querySelector("header")).toBeNull();
	expect(pillShows()).toEqual({ name: "Flux", look: "flux" });

	await show("bot:bot_scout");
	expect(pillShows()).toEqual({ name: "Scout", look: "mochi" });

	await show("bot:general");
	expect(pillShows()).toEqual({ name: "Assistant", look: "pip" });

	await show("bot:bot_gone");
	expect(pillShows().name).toBe("Deleted bot");
});

test("Open full chat opens that bot's chat on the Bots tab, its panel included", async () => {
	expect(await openFullChat()).toBe("/bots?bot=flux");
	await show("bot:bot_scout");
	expect(await openFullChat()).toBe("/bots?bot=bot_scout");
	await show("bot:general");
	expect(await openFullChat()).toBe("/bots");

	await act(async () => useGlobalChatStore.setState({ mode: "overlay" }));
	await show("bot:bot_scout");
	await click(pill());
	expect(await openFullChat()).toBe("/bots?bot=bot_scout&panel=details");
});

test("the pill opens the bot's panel as a sheet inside the overlay, the window buttons in reach", async () => {
	await click(pill());
	expect(pill()?.getAttribute("aria-expanded")).toBe("true");
	expect(panel()?.querySelector("h2")?.textContent).toBe("Flux");
	expect(
		Array.from(panel()?.querySelectorAll("[role='tab']") ?? []).map(
			(tab) => tab.textContent,
		),
	).toEqual(["Details", "Memory"]);
	expect(panel()?.textContent).toContain("FlowPilot");
	expect(panelButton("Open full chat")).toBeDefined();
	expect(panelButton("Close chat")).toBeDefined();
	expect(
		document.body
			.querySelector("[data-testid='chat-body']")
			?.closest("[inert]"),
	).not.toBeNull();

	await click(panelButton("Back to the chat"));
	expect(panel()).toBeNull();
	expect(pill()?.getAttribute("aria-expanded")).toBe("false");
});

test("another bot's chat starts with the panel closed", async () => {
	await click(pill());
	expect(panel()).not.toBeNull();
	await show("bot:bot_scout");
	expect(panel()).toBeNull();
	expect(pillShows().name).toBe("Scout");
});

test("the panel gives way when the open chat asks for you", async () => {
	await click(pill());
	await act(async () =>
		useGlobalChatStore.setState({
			toolPrompt: {
				id: "prompt-1",
				kind: "approval",
				conversationId: "flux-1",
				toolName: "call_app_event",
				title: "Approve app event execution",
				respond: () => {},
			},
		}),
	);
	expect(panel()).toBeNull();
});

test("Edit opens the bot's settings over the page while the overlay waits behind them", async () => {
	await show("bot:bot_scout");
	await click(pill());
	expect(panel()?.textContent).toContain("Research digest");
	await click(panelButton("Edit"));
	expect(controls.editor.open).toBe(true);
	expect(controls.editor.botId).toBe("bot_scout");
	expect(pill()?.closest(".invisible")).not.toBeNull();

	await act(async () => controls.editor.onOpenChange?.(false));
	expect(controls.editor.open).toBe(false);
	expect(pill()?.closest(".invisible")).toBeNull();
	expect(useGlobalChatStore.getState().mode).toBe("overlay");
});

test("Duplicate opens the copy on the Bots tab", async () => {
	await show("bot:bot_scout");
	await click(pill());
	await click(panelButton("Duplicate"));
	expect(cloneBot).toHaveBeenCalledWith("bot_scout");
	expect(controls.push).toHaveBeenLastCalledWith("/bots?bot=bot_scout_copy");
	expect(useGlobalChatStore.getState().mode).toBe("closed");
});

test("a bot on this computer moves through the Bots tab's review", async () => {
	await show("bot:bot_pebble");
	await click(pill());
	expect(panel()?.textContent).toContain("On this device");
	await click(panelButton("Move to your account"));
	expect(controls.push).toHaveBeenLastCalledWith(
		"/bots?bot=bot_pebble&move=local",
	);
});
