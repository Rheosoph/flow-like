// @vitest-environment happy-dom
import "fake-indexeddb/auto";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { IBot } from "../../state/backend-state/bot-state";
import {
	selectAutoMode,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import { BotChatActions } from "./bot-chat-actions";
import type { GlobalChatBotView } from "./use-global-chat-bot";

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (key: string, fallback?: unknown, options?: Record<string, unknown>) =>
			(typeof fallback === "string" ? fallback : key).replace(
				/\{\{(\w+)\}\}/g,
				(_, name: string) => String(options?.[name] ?? ""),
			),
		i18n: { language: "en" },
	}),
}));
vi.mock("./chat-activity", () => ({ ChatActivityMenu: () => null }));
vi.mock("./global-chat-history", () => ({ GlobalChatHistory: () => null }));
vi.mock("./chat-memory-settings", () => ({ ChatMemorySettings: () => null }));
vi.mock("./chat-model-settings", () => ({ ChatModelSettings: () => null }));

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	useGlobalChatStore.setState({ autoMode: false, botAutoMode: {} });
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
});

function view(botId: string | null, primary: string | null): GlobalChatBotView {
	return {
		botId,
		bot: { model: { primary, fallbacks: [] } } as unknown as IBot,
		name: "Scout",
		description: "",
		appearance: {},
	};
}

async function render(
	chatBot: GlobalChatBotView,
	conversationId: string,
	defaultModelLabel: string | null = null,
) {
	await act(async () => {
		useGlobalChatStore.setState({ activeConversationId: conversationId });
		root.render(
			<BotChatActions
				chatBot={chatBot}
				defaultModelLabel={defaultModelLabel}
				onOpenSettings={() => {}}
			/>,
		);
	});
}

const autoButton = () =>
	Array.from(host.querySelectorAll("button")).find(
		(button) => button.textContent === "Auto",
	);

describe("a bot's composer actions", () => {
	test("show no model when the bot pins one, since the chat can't change it", async () => {
		await render(view("bot_scout", "bit-sonnet"), "bot:bot_scout", "GPT Test");
		expect(host.querySelector("[title*='model']")).toBeNull();
		expect(host.textContent).not.toContain("GPT Test");
		expect(autoButton()).toBeDefined();
	});

	test("show the chat's default model the same way when the bot pins none", async () => {
		await render(view("bot_scout", null), "bot:bot_scout", "GPT Test");
		expect(host.textContent).toContain("GPT Test");
		expect(host.textContent).toContain(
			"Scout has no model of its own, so it uses the chat's default model",
		);
		expect(host.querySelector("[title*='no model of its own']")).not.toBeNull();
	});

	test("show Flux's default model too", async () => {
		await render(view(null, null), "flux-notes", "GPT Test");
		expect(host.textContent).toContain("GPT Test");
	});

	test("show no model while the chat's default isn't known", async () => {
		await render(view("bot_scout", null), "bot:bot_scout", null);
		expect(host.textContent).not.toContain("no model of its own");
	});

	test("Auto switches the open chat's owner only", async () => {
		await render(view("bot_scout", null), "bot:bot_scout");
		await act(async () => autoButton()?.click());
		const state = useGlobalChatStore.getState();
		expect(selectAutoMode(state, "bot:bot_scout")).toBe(true);
		expect(selectAutoMode(state, "flux-notes")).toBe(false);
		expect(autoButton()?.getAttribute("aria-pressed")).toBe("true");
	});
});
