// @vitest-environment happy-dom
import "fake-indexeddb/auto";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type { IBot } from "../../state/backend-state/bot-state";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { BotModelChip, pinnedBotModel } from "./bot-chat-parts";
import { GlobalChatHistory } from "./global-chat-history";
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
vi.mock("../bots-tab/editor/steps/use-bot-models", () => ({
	useBotModelOptions: () => ({
		options: [
			{ id: "bit-sonnet", label: "Claude Sonnet", local: false, free: false },
		],
		isLoading: false,
	}),
}));

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
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

describe("the model a bot's chat runs on", () => {
	test("is the bot's pinned primary, Flux's too; bots without one keep the picker", () => {
		expect(pinnedBotModel(view("bot_scout", "bit-sonnet"))).toBe("bit-sonnet");
		expect(pinnedBotModel(view("bot_scout", null))).toBeNull();
		expect(pinnedBotModel(view(null, "bit-sonnet"))).toBe("bit-sonnet");
		expect(pinnedBotModel(view(null, null))).toBeNull();
		expect(
			pinnedBotModel({ ...view("general", null), bot: undefined }),
		).toBeNull();
	});

	test("shows as a read-only chip with the model's name", async () => {
		await act(async () =>
			root.render(<BotModelChip botName="Scout" modelId="bit-sonnet" />),
		);
		expect(host.textContent).toContain("Claude Sonnet");
		expect(host.querySelector("button")).toBeNull();
		expect(host.querySelector("[title]")?.getAttribute("title")).toBe(
			"Chosen in Scout's settings",
		);

		await act(async () =>
			root.render(<BotModelChip botName="Scout" modelId="bit-retired" />),
		);
		expect(host.textContent).toContain("Unknown model (bit-retired)");
	});
});

describe("history and New chat", () => {
	test("belong to Flux; a bot's chat has neither", async () => {
		useGlobalChatStore.setState({ activeConversationId: "bot:bot_scout" });
		await act(async () => root.render(<GlobalChatHistory />));
		expect(host.querySelector("button")).toBeNull();

		await act(async () =>
			useGlobalChatStore.setState({ activeConversationId: "flux-1" }),
		);
		expect(host.querySelector("button[aria-label='New chat']")).not.toBeNull();
		expect(
			host.querySelector("button[aria-label='Chat history']"),
		).not.toBeNull();
	});
});
