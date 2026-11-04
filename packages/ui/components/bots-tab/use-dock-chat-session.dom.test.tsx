// @vitest-environment happy-dom
import "fake-indexeddb/auto";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import {
	afterAll,
	afterEach,
	beforeEach,
	describe,
	expect,
	test,
	vi,
} from "vitest";
import {
	LAST_FLUX_CONVERSATION_KEY,
	readLastFluxConversation,
	rememberGlobalChatConversation,
	useGlobalChatStore,
} from "../../state/global-chat/global-chat-store";
import type { DockSelection } from "./dock-selection";
import { useDockChatSession } from "./use-dock-chat-session";

const controls = vi.hoisted(() => ({
	params: new URLSearchParams(),
	restoreWait: undefined as Promise<void> | undefined,
	restoreConversation: vi.fn(),
	switchBot: vi.fn(),
}));
const { restoreConversation, switchBot } = controls;
const previousChat = useGlobalChatStore.getState();

vi.mock("next/navigation", () => ({
	useSearchParams: () => controls.params,
}));
vi.mock("../../state/global-chat/global-chat-stream", () => ({
	restoreGlobalChatConversation: controls.restoreConversation,
	switchGlobalChatBot: controls.switchBot,
}));

let root: ReturnType<typeof createRoot>;
let container: HTMLElement;

function Harness({
	botId,
	sourceSelection,
}: {
	botId: string | null;
	sourceSelection?: DockSelection;
}) {
	useDockChatSession(botId, sourceSelection);
	return null;
}

async function render(
	botId: string | null = null,
	sourceSelection?: DockSelection,
) {
	await act(async () =>
		root.render(createElement(Harness, { botId, sourceSelection })),
	);
}

function setUrl(href: string) {
	window.history.replaceState(null, "", href);
	controls.params = new URLSearchParams(window.location.search);
}

function activeConversation() {
	return useGlobalChatStore.getState().activeConversationId;
}

function urlSession() {
	return new URLSearchParams(window.location.search).get("session");
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	window.sessionStorage.clear();
	useGlobalChatStore.setState(previousChat, true);
	useGlobalChatStore.getState().loadConversation("previous-chat", []);
	setUrl("/bots?bot=flux");
	controls.restoreWait = undefined;
	restoreConversation.mockReset();
	switchBot.mockReset();
	restoreConversation.mockImplementation(
		async (conversationId: string, options?: { signal?: AbortSignal }) => {
			useGlobalChatStore.getState().setConversationLoading(true);
			await controls.restoreWait;
			if (options?.signal?.aborted) return false;
			useGlobalChatStore.getState().loadConversation(conversationId, []);
			rememberGlobalChatConversation(conversationId);
			return true;
		},
	);
	switchBot.mockImplementation(
		async (botId: string | null, options?: { signal?: AbortSignal }) => {
			if (options?.signal?.aborted) return;
			const conversationId = botId
				? `bot:${botId}`
				: (readLastFluxConversation() ?? "fresh-flux");
			useGlobalChatStore.getState().loadConversation(conversationId, []);
		},
	);
	container = window.document.createElement("div") as unknown as HTMLElement;
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
});

afterAll(() => {
	useGlobalChatStore.setState(previousChat, true);
	window.sessionStorage.clear();
});

describe("Flux conversation bookmarks", () => {
	test("an explicit session wins over the open conversation and the tab's last chat", async () => {
		rememberGlobalChatConversation("remembered-chat");
		setUrl("/bots?bot=flux&session=bookmarked-chat");
		await render();
		expect(activeConversation()).toBe("bookmarked-chat");
		expect(urlSession()).toBe("bookmarked-chat");
		expect(restoreConversation.mock.calls[0]?.[0]).toBe("bookmarked-chat");
		expect(switchBot).not.toHaveBeenCalled();
		expect(window.sessionStorage.getItem(LAST_FLUX_CONVERSATION_KEY)).toBe(
			"bookmarked-chat",
		);
	});

	test("a fresh mount restores the bookmark after another chat was used", async () => {
		await render();
		await act(async () => {
			useGlobalChatStore.getState().loadConversation("chosen-history-chat", []);
		});
		const bookmark = window.location.href;
		await act(async () => root.unmount());
		useGlobalChatStore.getState().loadConversation("another-chat", []);
		rememberGlobalChatConversation("another-chat");
		setUrl(bookmark);
		root = createRoot(container);
		await render();
		expect(activeConversation()).toBe("chosen-history-chat");
		expect(urlSession()).toBe("chosen-history-chat");
	});

	test("opening Flux, picking history and starting a new chat keep the URL current", async () => {
		rememberGlobalChatConversation("last-flux");
		setUrl("/bots?bot=flux&panel=memory&voice=1#latest");
		await render();
		expect(urlSession()).toBe("last-flux");
		await act(async () => {
			useGlobalChatStore.getState().loadConversation("history/a+b&c", []);
		});
		expect(urlSession()).toBe("history/a+b&c");
		await act(async () => useGlobalChatStore.getState().newConversation());
		expect(urlSession()).toBe(activeConversation());
		expect(urlSession()).not.toBe("history/a+b&c");
		const query = new URLSearchParams(window.location.search);
		expect(query.get("panel")).toBe("memory");
		expect(query.get("voice")).toBe("1");
		expect(window.location.hash).toBe("#latest");
	});

	test("an external session change wins before rendered search params catch up", async () => {
		setUrl("/bots?bot=flux&session=first-chat");
		await render();
		window.history.replaceState(null, "", "/bots?bot=flux&session=linked-chat");
		await act(async () => {
			useGlobalChatStore.getState().loadConversation("background-chat", []);
		});
		expect(urlSession()).toBe("linked-chat");
		controls.params = new URLSearchParams(window.location.search);
		await render();
		expect(activeConversation()).toBe("linked-chat");
		expect(urlSession()).toBe("linked-chat");
	});

	test("acknowledging a history URL write preserves the conversation already on screen", async () => {
		setUrl("/bots?bot=flux&session=first-chat");
		await render();
		const messages = [{ id: "unsaved" }] as never;
		await act(async () => {
			useGlobalChatStore.setState({
				activeConversationId: "history-chat",
				messages,
			});
		});
		expect(urlSession()).toBe("history-chat");
		controls.params = new URLSearchParams(window.location.search);
		await render();
		expect(useGlobalChatStore.getState().messages).toBe(messages);
		expect(restoreConversation).toHaveBeenCalledTimes(1);
		await act(async () => useGlobalChatStore.getState().newConversation());
		const fresh = activeConversation();
		controls.params = new URLSearchParams(window.location.search);
		await render();
		expect(urlSession()).toBe(fresh);
		expect(activeConversation()).toBe(fresh);
		expect(restoreConversation).toHaveBeenCalledTimes(1);
	});

	test("an optimistic switch to Flux waits for its own URL before adding a session", async () => {
		setUrl("/bots?bot=scout");
		await render(null, { type: "system", system: "flux" });
		expect(window.location.search).toBe("?bot=scout");
		setUrl("/bots?bot=flux");
		await render(null, { type: "system", system: "flux" });
		expect(urlSession()).toBe("fresh-flux");
	});

	test("leaving Flux aborts a pending restore and keeps the next conversation's URL", async () => {
		let finishRestore!: () => void;
		controls.restoreWait = new Promise<void>((resolve) => {
			finishRestore = resolve;
		});
		setUrl("/bots?bot=flux&session=slow-chat");
		await render();
		const signal = restoreConversation.mock.calls[0]?.[1]?.signal;
		expect(signal?.aborted).toBe(false);
		setUrl("/bots?conversation=room-1");
		await act(async () => root.render(null));
		expect(signal?.aborted).toBe(true);
		await act(async () => finishRestore());
		expect(activeConversation()).toBe("previous-chat");
		expect(window.location.search).toBe("?conversation=room-1");
	});

	test("a newer linked session aborts the earlier restore on the same bot", async () => {
		let finishRestore!: () => void;
		controls.restoreWait = new Promise<void>((resolve) => {
			finishRestore = resolve;
		});
		setUrl("/bots?bot=flux&session=slow-chat");
		await render();
		const previousSignal = restoreConversation.mock.calls[0]?.[1]?.signal;
		controls.restoreWait = undefined;
		setUrl("/bots?bot=flux&session=latest-chat");
		await render();
		expect(previousSignal?.aborted).toBe(true);
		expect(activeConversation()).toBe("latest-chat");
		await act(async () => finishRestore());
		expect(activeConversation()).toBe("latest-chat");
		expect(urlSession()).toBe("latest-chat");
	});

	test("a bot opens its own continuous conversation without adding a Flux session", async () => {
		setUrl("/bots?bot=bot_scout");
		await render("bot_scout");
		expect(activeConversation()).toBe("bot:bot_scout");
		expect(switchBot.mock.calls[0]?.[0]).toBe("bot_scout");
		expect(restoreConversation).not.toHaveBeenCalled();
		expect(window.location.search).toBe("?bot=bot_scout");
	});

	const aliases: [string, string, DockSelection][] = [
		["bot ID", "bot=bot_flux", { type: "bot", botId: "bot_flux" }],
		[
			"direct conversation",
			"conversation=flux-dm",
			{ type: "conversation", conversationId: "flux-dm" },
		],
	];
	for (const [name, query, selection] of aliases) {
		test(`Flux opened through its ${name} restores and updates its session`, async () => {
			setUrl(`/bots?${query}&session=linked-flux`);
			await render(null, selection);
			expect(activeConversation()).toBe("linked-flux");
			expect(restoreConversation.mock.calls[0]?.[0]).toBe("linked-flux");
			await act(async () => {
				useGlobalChatStore.getState().loadConversation("other-flux", []);
			});
			expect(window.location.search).toBe(`?${query}&session=other-flux`);
		});
	}

	test("a pending restore leaves an unrelated live URL intact before the dock unmounts", async () => {
		let finishRestore!: () => void;
		controls.restoreWait = new Promise<void>((resolve) => {
			finishRestore = resolve;
		});
		setUrl("/bots?bot=flux&session=slow-chat");
		await render();
		window.history.replaceState(null, "", "/bots?conversation=room-2#latest");
		await act(async () => finishRestore());
		expect(window.location.search).toBe("?conversation=room-2");
		expect(window.location.hash).toBe("#latest");
	});

	test("a bot conversation ID in Flux's session parameter is not restored", async () => {
		setUrl("/bots?bot=flux&session=bot%3Abot_scout");
		await render();
		expect(restoreConversation).not.toHaveBeenCalled();
		expect(switchBot.mock.calls[0]?.[0]).toBeNull();
		expect(activeConversation()).toBe("fresh-flux");
		expect(urlSession()).toBe("fresh-flux");
	});

	test("a failed bookmark stays in the URL until the user picks another history entry", async () => {
		let failRestore!: (error: Error) => void;
		controls.restoreWait = new Promise<void>((_resolve, reject) => {
			failRestore = reject;
		});
		const error = new Error("History unavailable");
		const warning = vi.spyOn(console, "warn").mockImplementation(() => {});
		try {
			setUrl("/bots?bot=flux&session=unavailable-chat");
			await render();
			await act(async () => failRestore(error));
			expect(urlSession()).toBe("unavailable-chat");
			expect(activeConversation()).toBe("previous-chat");
			expect(warning).toHaveBeenCalledWith(
				"[BotChat] Opening the linked conversation failed:",
				error,
			);
			await act(async () => {
				useGlobalChatStore.getState().loadConversation("selected-history", []);
			});
			expect(urlSession()).toBe("selected-history");
		} finally {
			warning.mockRestore();
		}
	});
});
