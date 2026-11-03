// @vitest-environment happy-dom
import "fake-indexeddb/auto";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { IRole } from "../../lib/schema/llm/history";
import { useAuthStatusStore, useBackendStore } from "../../state/backend-state";
import type { IBot } from "../../state/backend-state/bot-state";
import {
	chatActivityKey,
	useChatActivityStore,
} from "../../state/chat-activity-store";
import { forgetMovedConversation } from "../../state/global-chat/global-chat-bots";
import { globalChatDb } from "../../state/global-chat/global-chat-db";
import {
	LAST_CONVERSATION_KEY,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import {
	makeGlobalChatMessage,
	persistGlobalChatMessage,
	restoreGlobalChatConversation,
} from "../../state/global-chat/global-chat-stream";
import { GlobalChatOverlay } from "./global-chat-overlay";

vi.mock("@flow-like/locales", async (importOriginal) => ({
	...(await importOriginal<typeof import("@flow-like/locales")>()),
	useTranslation: () => ({
		t: (key: string, fallback?: unknown) =>
			typeof fallback === "string" ? fallback : key,
		i18n: { language: "en" },
	}),
}));
vi.mock("next/navigation", () => ({
	useRouter: () => ({ push: () => {} }),
	usePathname: () => "/library",
}));
vi.mock("./global-chat-body", () => ({
	GlobalChatBody: () => <div data-testid="chat-body" />,
}));
vi.mock("../bots-tab/editor/bot-editor-dialog", () => ({
	BotEditorDialog: () => null,
}));
vi.mock("../../hooks/use-bot-expression", () => ({
	useBotExpression: () => ({ expression: "neutral", status: "idle" }),
}));
vi.mock("../bots/bot-avatar", () => ({
	BotAvatar: () => <span />,
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

function flux(id: string, local?: boolean): IBot {
	return { ...assistant(id, local), system: "flux" };
}

function assistant(id: string, local?: boolean): IBot {
	return {
		id,
		owner_id: local ? "local" : "me",
		system: "general",
		preset_id: null,
		name: null,
		description: null,
		instructions: null,
		appearance: {} as IBot["appearance"],
		model: { primary: null, fallbacks: [] },
		classification: {} as IBot["classification"],
		capabilities: ["internet"],
		profile_id: null,
		location: local ? "desktop" : "server",
		device_id: null,
		created_at: "",
		updated_at: "",
		...(local ? { local } : {}),
	};
}

const LEGACY = "bot:general";
let root: Root;
let host: HTMLDivElement;

async function settle() {
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 30));
	});
}

/** What a chat said before another row took it over: saved, queued, pointed at and unread. */
async function legacyChat(conversationId = LEGACY) {
	const said = makeGlobalChatMessage(
		IRole.User,
		"Plan my week",
		conversationId,
	);
	await persistGlobalChatMessage(said);
	sessionStorage.setItem(LAST_CONVERSATION_KEY, conversationId);
	useGlobalChatStore.getState().enqueueMessage({
		conversationId,
		content: "And next week",
	});
	const target = { kind: "flowpilot" as const, sessionId: conversationId };
	const unread = {
		id: chatActivityKey(target),
		target,
		messageId: "reply-1",
		title: "Assistant",
		preview: "Here is your week.",
		createdAt: Date.now(),
	};
	localStorage.setItem(
		"flow-like:chat-activity:v1",
		JSON.stringify({ entries: [unread], receipts: [] }),
	);
	useChatActivityStore.setState({ entries: [unread] });
	return said;
}

async function mount(backend: unknown, signedIn: boolean) {
	useAuthStatusStore.setState({ signedIn });
	useBackendStore.setState({ backend: backend as never });
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
	await settle();
	return client;
}

async function opened(conversationId: string) {
	await act(async () => {
		await restoreGlobalChatConversation(conversationId);
	});
	return useGlobalChatStore.getState().activeConversationId;
}

async function savedUnder(conversationId: string) {
	return globalChatDb.messages
		.where("sessionId")
		.equals(conversationId)
		.count();
}

function where() {
	const state = useGlobalChatStore.getState();
	return {
		pointer: sessionStorage.getItem(LAST_CONVERSATION_KEY),
		queue: state.queue.map((entry) => entry.conversationId),
		activity: useChatActivityStore
			.getState()
			.entries.map((entry) => entry.target.sessionId),
	};
}

beforeEach(async () => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	await globalChatDb.messages.clear();
	await globalChatDb.sessions.clear();
	sessionStorage.clear();
	localStorage.clear();
	// Each test starts from a chat made before any row took `general` over.
	forgetMovedConversation(LEGACY);
	useChatActivityStore.setState({ entries: [] });
	useGlobalChatStore.setState({
		mode: "closed",
		activeConversationId: "flux-1",
		messages: [],
		runs: {},
		queue: [],
	});
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	useBackendStore.setState({ backend: null });
});

describe("the Assistant's chats from before it had a row", () => {
	test("wait while the list can't be read", async () => {
		await legacyChat();
		await mount(
			{
				botState: {
					listBots: async () => {
						throw new Error("bots unavailable");
					},
				},
			},
			true,
		);
		expect(await savedUnder(LEGACY)).toBe(1);
		expect(where()).toEqual({
			pointer: LEGACY,
			queue: [LEGACY],
			activity: [LEGACY],
		});
	});

	test("wait while the list loads", async () => {
		await legacyChat();
		await mount({ botState: { listBots: () => new Promise(() => {}) } }, true);
		expect(await savedUnder(LEGACY)).toBe(1);
		expect(where().pointer).toBe(LEGACY);
	});

	test("signed in, move to the account's Assistant with their pointer, queue and unread reply", async () => {
		await legacyChat();
		let listFails = false;
		const client = await mount(
			{
				botState: {
					listBots: async () => {
						if (listFails) throw new Error("bots unavailable");
						return [
							assistant("bot_general_here", true),
							assistant("bot_general_acct"),
						];
					},
				},
			},
			true,
		);
		const row = "bot:bot_general_acct";
		expect(await savedUnder(LEGACY)).toBe(0);
		expect(await savedUnder(row)).toBe(1);
		expect(where()).toEqual({ pointer: row, queue: [row], activity: [row] });
		expect(await opened(LEGACY)).toBe(row);

		listFails = true;
		await act(async () => {
			await client.refetchQueries();
		});
		await settle();
		expect(await opened(LEGACY)).toBe(row);
	});

	test("signed out on this computer, move to its Assistant row", async () => {
		await legacyChat();
		await mount(
			{
				botState: {
					local: {},
					listBots: async () => [assistant("bot_general_desk", true)],
				},
			},
			false,
		);
		expect(await savedUnder(LEGACY)).toBe(0);
		expect(await savedUnder("bot:bot_general_desk")).toBe(1);
	});

	test("signed in, a hidden local Assistant's chat moves to the account's; a hidden Flux has none", async () => {
		const quiet = "bot:bot_general_quiet";
		await legacyChat(quiet);
		await mount(
			{
				botState: {
					listBots: async () => [
						assistant("bot_general_acct2"),
						flux("bot_flux_acct2"),
					],
					local: {
						list: async () => [
							assistant("bot_general_quiet", true),
							flux("bot_flux_quiet", true),
						],
					},
				},
			},
			true,
		);
		const row = "bot:bot_general_acct2";
		expect(await savedUnder(quiet)).toBe(0);
		expect(await savedUnder(row)).toBe(1);
		expect(where()).toEqual({ pointer: row, queue: [row], activity: [row] });
		expect(await opened(quiet)).toBe(row);
		expect(await opened("bot:bot_flux_quiet")).toBe("bot:bot_flux_quiet");

		await act(async () => root.unmount());
		root = createRoot(host);
		await mount(
			{
				botState: {
					local: {},
					listBots: async () => [assistant("bot_general_quiet", true)],
				},
			},
			false,
		);
		expect(await opened(quiet)).toBe(quiet);
	});

	test("an edited local Assistant stays listed and keeps its own chat until it moves", async () => {
		const edited = "bot:bot_general_edited";
		await legacyChat(edited);
		const local = assistant("bot_general_edited", true);
		await mount(
			{
				botState: {
					listBots: async () => [assistant("bot_general_acct3"), local],
					local: { list: async () => [local] },
				},
			},
			true,
		);
		expect(await savedUnder(edited)).toBe(1);
		expect(await opened(edited)).toBe(edited);
	});

	test("signed out without any row, the Assistant is `general` again", async () => {
		await mount(
			{ botState: { listBots: async () => [assistant("bot_general_old")] } },
			true,
		);
		expect(await opened(LEGACY)).toBe("bot:bot_general_old");
		await act(async () => root.unmount());
		root = createRoot(host);

		await mount({}, false);
		expect(await opened(LEGACY)).toBe(LEGACY);
	});
});
