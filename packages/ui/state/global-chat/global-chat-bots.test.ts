// @vitest-environment happy-dom
import "fake-indexeddb/auto";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { IRole } from "../../lib/schema/llm/history";
import {
	ASSISTANT_BOT_ID,
	botConversationId,
	conversationBotId,
	desktopChatBotArgs,
	webChatBotBody,
} from "./global-chat-bots";
import { globalChatDb, listFluxChatSessions } from "./global-chat-db";
import {
	deleteGlobalChatMemory,
	listGlobalChatMemories,
} from "./global-chat-memory";
import {
	LAST_CONVERSATION_KEY,
	LAST_FLUX_CONVERSATION_KEY,
	selectActiveBotId,
	useGlobalChatStore,
} from "./global-chat-store";
import {
	driveGlobalChatStream,
	forgetBotChat,
	makeGlobalChatMessage,
	persistGlobalChatMessage,
	persistGlobalChatSession,
	readActiveRuns,
	restoreGlobalChatConversation,
	setActiveRun,
	switchGlobalChatBot,
	tauriStart,
} from "./global-chat-stream";
import { webGlobalChatStart } from "./global-chat-web-transport";

const tauri = vi.hoisted(() => ({
	invoke: vi.fn(async (_command: string, _args: Record<string, unknown>) => ({
		message: "",
	})),
}));
vi.mock("@tauri-apps/api/core", () => ({
	Channel: class {
		onmessage?: (chunk: string) => void;
	},
	invoke: tauri.invoke,
}));

beforeEach(async () => {
	await globalChatDb.messages.clear();
	await globalChatDb.sessions.clear();
	sessionStorage.clear();
	useGlobalChatStore.setState({
		activeConversationId: "flux-fresh",
		messages: [],
		runs: {},
		queue: [],
		draft: null,
		nativeDrafts: [],
		isStreaming: false,
		streamingMessages: [],
	});
});

afterEach(() => {
	vi.unstubAllGlobals();
	tauri.invoke.mockClear();
});

async function saveTurn(conversationId: string, text: string) {
	const message = makeGlobalChatMessage(IRole.User, text, conversationId);
	await persistGlobalChatMessage(message);
	await persistGlobalChatSession(conversationId, text);
	return message;
}

function shown() {
	const state = useGlobalChatStore.getState();
	return {
		conversationId: state.activeConversationId,
		botId: selectActiveBotId(state),
		texts: state.messages.map((message) => message.inner.content),
	};
}

/** A turn whose stream the test feeds and ends by hand. */
function liveTurn(conversationId: string) {
	const reply = makeGlobalChatMessage(IRole.Assistant, "", conversationId);
	const control = {
		emit: (_chunk: string) => {},
		finish: () => {},
	};
	const done = driveGlobalChatStream({
		responseMessage: reply,
		label: "Turn",
		start: (onChunk) =>
			new Promise((resolve) => {
				control.emit = onChunk;
				control.finish = () => resolve({});
			}),
	});
	return { reply, control, done };
}

describe("a bot's one conversation", () => {
	test("is named by the bot id; Assistant without an account row is `general`", () => {
		expect(botConversationId("bot_scout")).toBe("bot:bot_scout");
		expect(botConversationId(ASSISTANT_BOT_ID)).toBe("bot:general");
		expect(conversationBotId("bot:bot_scout")).toBe("bot_scout");
		expect(conversationBotId("ckq1flux")).toBeNull();
		expect(conversationBotId("bot:")).toBeNull();
	});

	test("selecting a bot opens its conversation with everything said before", async () => {
		await saveTurn("bot:bot_scout", "What's new in robotics?");
		await switchGlobalChatBot("bot_scout");
		expect(shown()).toEqual({
			conversationId: "bot:bot_scout",
			botId: "bot_scout",
			texts: ["What's new in robotics?"],
		});

		await switchGlobalChatBot("bot_writer");
		expect(shown()).toEqual({
			conversationId: "bot:bot_writer",
			botId: "bot_writer",
			texts: [],
		});

		await switchGlobalChatBot("bot_scout");
		expect(shown().texts).toEqual(["What's new in robotics?"]);
	});

	test("Flux returns to its own last conversation; New chat starts it over", async () => {
		const first = await saveTurn("flux-launch", "Plan the launch app");
		useGlobalChatStore.getState().loadConversation("flux-launch", [first]);

		await switchGlobalChatBot("bot_scout");
		expect(sessionStorage.getItem(LAST_FLUX_CONVERSATION_KEY)).toBe(
			"flux-launch",
		);
		await switchGlobalChatBot(null);
		expect(shown()).toEqual({
			conversationId: "flux-launch",
			botId: null,
			texts: ["Plan the launch app"],
		});

		useGlobalChatStore.getState().newConversation();
		const fresh = shown().conversationId;
		await switchGlobalChatBot("bot_scout");
		await switchGlobalChatBot(null);
		expect(shown().conversationId).not.toBe("flux-launch");
		expect(shown().conversationId).not.toBe(fresh);
		expect(shown().botId).toBeNull();
	});

	test("a Flux chat a reload left fresh picks up Flux's last one, unless a draft owns it", async () => {
		await saveTurn("flux-launch", "Plan the launch app");
		sessionStorage.setItem(LAST_FLUX_CONVERSATION_KEY, "flux-launch");

		useGlobalChatStore.setState({ draft: { prompt: "Create an app" } });
		await switchGlobalChatBot(null);
		expect(shown().conversationId).toBe("flux-fresh");

		useGlobalChatStore.setState({ draft: null });
		await switchGlobalChatBot(null);
		expect(shown().conversationId).toBe("flux-launch");
	});

	test("the newest switch wins", async () => {
		await saveTurn("bot:bot_scout", "Scout's turn");
		await Promise.all([
			switchGlobalChatBot("bot_scout"),
			switchGlobalChatBot("bot_writer"),
		]);
		expect(shown().conversationId).toBe("bot:bot_writer");
	});

	test("a run keeps its bot: switching bots mid-stream never moves its reply", async () => {
		await switchGlobalChatBot("bot_scout");
		const { reply, control, done } = liveTurn("bot:bot_scout");
		const run = useGlobalChatStore.getState().runs[reply.id];
		expect(desktopChatBotArgs(run?.conversationId ?? "")).toEqual({
			botId: "bot_scout",
		});

		await switchGlobalChatBot("bot_writer");
		control.emit("Robots learned to fold laundry.");
		control.finish();
		await done;

		expect(shown()).toEqual({
			conversationId: "bot:bot_writer",
			botId: "bot_writer",
			texts: [],
		});
		const saved = await globalChatDb.messages.get(reply.id);
		expect(saved?.sessionId).toBe("bot:bot_scout");
		expect(saved?.inner.content).toBe("Robots learned to fold laundry.");
	});
});

/** Saved rows of a conversation that a test hands over (or refuses) when it chooses. */
function heldReads() {
	const pending = new Map<
		string,
		{ resolve: (rows: IMessage[]) => void; reject: (error: Error) => void }
	>();
	const spy = vi
		.spyOn(globalChatDb.messages, "where")
		.mockImplementation((() => ({
			equals: (conversationId: string) => ({
				sortBy: () =>
					new Promise<IMessage[]>((resolve, reject) =>
						pending.set(conversationId, { resolve, reject }),
					),
			}),
		})) as never);
	return {
		spy,
		reading: (conversationId: string) => pending.has(conversationId),
		hand: (conversationId: string, rows: IMessage[] = []) =>
			pending.get(conversationId)?.resolve(rows),
		refuse: (conversationId: string, error: Error) =>
			pending.get(conversationId)?.reject(error),
	};
}

const loading = () => useGlobalChatStore.getState().conversationLoading;

describe("the chat's loading state while a conversation is read", () => {
	test("a bot switch holds it, with the previous chat still in the store, until the bot's conversation lands", async () => {
		const reads = heldReads();
		try {
			const switching = switchGlobalChatBot("bot_scout");
			expect(loading()).toBe(true);
			expect(shown().conversationId).toBe("flux-fresh");

			await vi.waitFor(() => expect(reads.reading("bot:bot_scout")).toBe(true));
			reads.hand("bot:bot_scout", [
				makeGlobalChatMessage(IRole.User, "Earlier", "bot:bot_scout"),
			]);
			await switching;
			expect(loading()).toBe(false);
			expect(shown()).toEqual({
				conversationId: "bot:bot_scout",
				botId: "bot_scout",
				texts: ["Earlier"],
			});
		} finally {
			reads.spy.mockRestore();
		}
	});

	test("only the newest switch ends it, whichever read comes back first", async () => {
		const reads = heldReads();
		try {
			const first = switchGlobalChatBot("bot_scout");
			const second = switchGlobalChatBot("bot_writer");
			await vi.waitFor(() => {
				expect(reads.reading("bot:bot_scout")).toBe(true);
				expect(reads.reading("bot:bot_writer")).toBe(true);
			});

			reads.hand("bot:bot_scout");
			await first;
			expect(loading()).toBe(true);
			expect(shown().conversationId).toBe("flux-fresh");

			reads.hand("bot:bot_writer");
			await second;
			expect(loading()).toBe(false);
			expect(shown().conversationId).toBe("bot:bot_writer");
		} finally {
			reads.spy.mockRestore();
		}
	});

	test("a read that comes back after a newer chat took the screen leaves it loaded and untouched", async () => {
		const reads = heldReads();
		try {
			const switching = switchGlobalChatBot("bot_scout");
			await vi.waitFor(() => expect(reads.reading("bot:bot_scout")).toBe(true));
			useGlobalChatStore.getState().newConversation();
			reads.hand("bot:bot_scout");
			await switching;
			expect(loading()).toBe(false);
			expect(shown().botId).toBeNull();
			expect(shown().conversationId).not.toBe("bot:bot_scout");
		} finally {
			reads.spy.mockRestore();
		}
	});

	test("a switch to the chat already shown, or to a fresh Flux chat, never shows it", async () => {
		const reads = heldReads();
		try {
			await switchGlobalChatBot(null);
			expect(loading()).toBe(false);
			useGlobalChatStore.getState().loadConversation("bot:bot_scout", []);
			await switchGlobalChatBot("bot_scout");
			expect(loading()).toBe(false);
			await switchGlobalChatBot(null);
			expect(loading()).toBe(false);
			expect(reads.reading("bot:bot_scout")).toBe(false);
		} finally {
			reads.spy.mockRestore();
		}
	});

	test("restoring a conversation from history holds it too, and a failed read ends it", async () => {
		const reads = heldReads();
		try {
			const restoring = restoreGlobalChatConversation("bot:bot_scout");
			expect(loading()).toBe(true);
			await vi.waitFor(() => expect(reads.reading("bot:bot_scout")).toBe(true));
			reads.hand("bot:bot_scout");
			await restoring;
			expect(loading()).toBe(false);
			expect(shown().conversationId).toBe("bot:bot_scout");

			const failing = restoreGlobalChatConversation("bot:bot_writer");
			expect(loading()).toBe(true);
			await vi.waitFor(() =>
				expect(reads.reading("bot:bot_writer")).toBe(true),
			);
			reads.refuse("bot:bot_writer", new Error("Storage is gone"));
			await expect(failing).rejects.toThrow("Storage is gone");
			expect(loading()).toBe(false);
			expect(shown().conversationId).toBe("bot:bot_scout");
		} finally {
			reads.spy.mockRestore();
		}
	});

	test("the restore a freshly mounted chat does on its own stays out of sight", async () => {
		const reads = heldReads();
		try {
			const restoring = restoreGlobalChatConversation("flux-launch", {
				skipIfBusy: true,
			});
			expect(loading()).toBe(false);
			await vi.waitFor(() => expect(reads.reading("flux-launch")).toBe(true));
			reads.hand("flux-launch");
			await restoring;
			expect(loading()).toBe(false);
		} finally {
			reads.spy.mockRestore();
		}
	});

	test("a mount restore that overtakes a visible switch doesn't leave the chat loading", async () => {
		const reads = heldReads();
		try {
			const switching = switchGlobalChatBot("bot_scout");
			await vi.waitFor(() => expect(reads.reading("bot:bot_scout")).toBe(true));
			const restoring = restoreGlobalChatConversation("flux-launch", {
				skipIfBusy: true,
			});
			await vi.waitFor(() => expect(reads.reading("flux-launch")).toBe(true));
			reads.hand("bot:bot_scout");
			await switching;
			reads.hand("flux-launch");
			await restoring;
			expect(loading()).toBe(false);
		} finally {
			reads.spy.mockRestore();
		}
	});
});

describe("start payloads", () => {
	test("the desktop sends the turn's bot as `botId`, and `flux` for Flux", async () => {
		await tauriStart("global_chat", {
			userPrompt: "Hi",
			...desktopChatBotArgs("bot:bot_scout"),
		})(() => {});
		expect(tauri.invoke).toHaveBeenLastCalledWith(
			"global_chat",
			expect.objectContaining({ userPrompt: "Hi", botId: "bot_scout" }),
		);

		await tauriStart("global_chat", {
			userPrompt: "Hi",
			...desktopChatBotArgs(botConversationId(ASSISTANT_BOT_ID)),
		})(() => {});
		expect(tauri.invoke.mock.lastCall?.[1]).toMatchObject({
			botId: "general",
		});

		await tauriStart("global_chat", {
			userPrompt: "Hi",
			...desktopChatBotArgs("flux-launch"),
		})(() => {});
		expect(tauri.invoke.mock.lastCall?.[1]).toMatchObject({ botId: "flux" });
	});

	test("the web body sends it as `bot_id`, and `flux` for Flux", async () => {
		const bodies: Record<string, unknown>[] = [];
		const channel = {
			channel_id: "run-channel",
			expires_at: 4_000_000_000,
			transport: { type: "in_process" },
		};
		vi.stubGlobal(
			"fetch",
			vi.fn(async (_url: string, init: RequestInit) => {
				bodies.push(JSON.parse(String(init.body)));
				return new Response(
					`event: run\ndata: ${JSON.stringify({ runId: "server-run", channel })}\n\nevent: final\ndata: {}\n\n`,
					{ status: 200, headers: { "content-type": "text/event-stream" } },
				);
			}),
		);
		const send = (conversationId: string) =>
			webGlobalChatStart({
				baseUrl: "https://api.example.test",
				body: { user_prompt: "Hi", ...webChatBotBody(conversationId) },
			})(() => {});

		await send("bot:bot_acct_assistant");
		await send("flux-launch");
		expect(bodies[0]).toMatchObject({
			user_prompt: "Hi",
			bot_id: "bot_acct_assistant",
		});
		expect(bodies[1]).toMatchObject({ user_prompt: "Hi", bot_id: "flux" });
	});
});

describe("memory of a chat", () => {
	test("is the bot's own in a bot's chat and the profile's for Flux", async () => {
		const urls: string[] = [];
		vi.stubGlobal(
			"fetch",
			vi.fn(async (url: string) => {
				urls.push(url);
				return new Response("[]", { status: 200 });
			}),
		);
		await listGlobalChatMemories({ profileId: "p1", botId: "bot_scout" });
		await deleteGlobalChatMemory({ profileId: "p1", botId: "bot_scout" }, "m1");
		await listGlobalChatMemories({ profileId: "p1", botId: null });

		const [scoutList, scoutDelete, fluxList] = urls.map((url) => new URL(url));
		expect(scoutList?.pathname).toBe("/api/v1/ai/global-chat/memory/entries");
		expect(Object.fromEntries(scoutList?.searchParams ?? [])).toEqual({
			profile_id: "p1",
			bot_id: "bot_scout",
		});
		expect(scoutDelete?.pathname).toBe("/api/v1/ai/global-chat/memory/m1");
		expect(scoutDelete?.searchParams.get("bot_id")).toBe("bot_scout");
		expect(Object.fromEntries(fluxList?.searchParams ?? [])).toEqual({
			profile_id: "p1",
		});
	});
});

describe("Flux's history", () => {
	test("lists Flux's own sessions, pinned or not, and never a bot's conversation", async () => {
		await saveTurn("flux-old", "An old Flux chat");
		await saveTurn("bot:bot_scout", "Scout chat");
		await saveTurn("bot:general", "Assistant chat");
		await globalChatDb.sessions.update("bot:bot_scout", {
			pinnedAt: Date.now(),
		});
		await globalChatDb.sessions.update("flux-old", { pinnedAt: Date.now() });
		await saveTurn("flux-new", "A new Flux chat");

		const sessions = await listFluxChatSessions(200);
		expect(sessions.map((session) => session.id)).toEqual([
			"flux-old",
			"flux-new",
		]);
	});
});

describe("a deleted bot's chat", () => {
	test("leaves for Flux's last conversation and is neither restored nor drained again", async () => {
		const plan = await saveTurn("flux-launch", "Plan the launch app");
		useGlobalChatStore.getState().loadConversation("flux-launch", [plan]);
		await switchGlobalChatBot("bot_scout");
		sessionStorage.setItem(LAST_CONVERSATION_KEY, "bot:bot_scout");
		useGlobalChatStore.getState().enqueueMessage({
			conversationId: "bot:bot_scout",
			content: "Still there?",
		});

		await forgetBotChat("bot_scout");
		expect(shown()).toEqual({
			conversationId: "flux-launch",
			botId: null,
			texts: ["Plan the launch app"],
		});
		expect(sessionStorage.getItem(LAST_CONVERSATION_KEY)).toBe("flux-launch");
		expect(useGlobalChatStore.getState().queue).toEqual([]);
	});

	test("isn't restored after a reload even when another chat is open", async () => {
		sessionStorage.setItem(LAST_CONVERSATION_KEY, "bot:bot_scout");
		await forgetBotChat("bot_scout");
		expect(sessionStorage.getItem(LAST_CONVERSATION_KEY)).toBeNull();
		expect(shown().conversationId).toBe("flux-fresh");
	});
});

describe("rebindBotChat", () => {
	test("moves a bot's conversation to its new id: rows, transcript, queue, runs and pointers", async () => {
		const first = await saveTurn("bot:local_painter", "Paint me a fox");
		await switchGlobalChatBot("local_painter");
		useGlobalChatStore.getState().enqueueMessage({
			conversationId: "bot:local_painter",
			content: "And a bear",
		});
		const { reply, control, done } = liveTurn("bot:local_painter");
		setActiveRun("bot:local_painter", reply.id);

		await useGlobalChatStore
			.getState()
			.rebindBotChat("local_painter", "bot_acct_painter");

		const state = useGlobalChatStore.getState();
		expect(shown()).toMatchObject({
			conversationId: "bot:bot_acct_painter",
			botId: "bot_acct_painter",
			texts: ["Paint me a fox"],
		});
		expect(state.messages[0]?.sessionId).toBe("bot:bot_acct_painter");
		expect(state.queue.map((entry) => entry.conversationId)).toEqual([
			"bot:bot_acct_painter",
		]);
		expect(state.runs[reply.id]?.conversationId).toBe("bot:bot_acct_painter");
		expect(state.isStreaming).toBe(true);
		expect(readActiveRuns().map((run) => run.conversationId)).toEqual([
			"bot:bot_acct_painter",
		]);
		expect(sessionStorage.getItem(LAST_CONVERSATION_KEY)).toBe(
			"bot:bot_acct_painter",
		);
		expect(
			await globalChatDb.sessions.get("bot:local_painter"),
		).toBeUndefined();
		expect(
			(await globalChatDb.sessions.get("bot:bot_acct_painter"))?.summarization,
		).toBe("Paint me a fox");
		expect((await globalChatDb.messages.get(first.id))?.sessionId).toBe(
			"bot:bot_acct_painter",
		);

		control.emit("A fox, in oils.");
		control.finish();
		await done;
		expect((await globalChatDb.messages.get(reply.id))?.sessionId).toBe(
			"bot:bot_acct_painter",
		);
		expect(shown().texts).toEqual(["Paint me a fox", "A fox, in oils."]);

		const settled = useGlobalChatStore.getState();
		await settled.rebindBotChat("local_painter", "bot_acct_painter");
		expect(useGlobalChatStore.getState()).toBe(settled);
		expect(
			await globalChatDb.messages
				.where("sessionId")
				.equals("bot:bot_acct_painter")
				.count(),
		).toBe(2);
		expect(
			await globalChatDb.messages
				.where("sessionId")
				.equals("bot:local_painter")
				.count(),
		).toBe(0);
	});

	test("moving the bot on screen while it replies keeps the cards and review its run waits on", async () => {
		await saveTurn("bot:local_sketcher", "Paint me a fox");
		await switchGlobalChatBot("local_sketcher");
		const { reply, control, done } = liveTurn("bot:local_sketcher");
		control.emit("Sketching");
		await vi.waitFor(async () =>
			expect(await globalChatDb.messages.get(reply.id)).toBeDefined(),
		);
		const store = useGlobalChatStore.getState();
		store.addInteractions([
			{
				id: "choice-1",
				name: "Pick a frame",
				description: "",
				interaction_type: "single_choice" as never,
				status: "pending",
				ttl_seconds: 60,
				expires_at: Date.now() + 60_000,
			},
		]);
		store.addInlineAppChat(
			{ appId: "app_atlas", eventId: "chat", name: "Atlas" },
			reply.id,
		);
		store.setPendingComponents(reply.id, { components: [], surfaceId: "s1" });

		await useGlobalChatStore
			.getState()
			.rebindBotChat("local_sketcher", "bot_acct_sketcher");
		const state = useGlobalChatStore.getState();
		expect(state.activeConversationId).toBe("bot:bot_acct_sketcher");
		expect(state.activeInteractions.map((item) => item.id)).toEqual([
			"choice-1",
		]);
		expect(state.inlineAppChats.map((chat) => chat.appId)).toEqual([
			"app_atlas",
		]);
		expect(state.pendingComponents?.surfaceId).toBe("s1");
		expect(state.messages.map((message) => message.id)).not.toContain(reply.id);
		expect(state.streamingMessages.map((message) => message.id)).toEqual([
			reply.id,
		]);

		control.finish();
		await done;
		expect(shown().texts).toEqual(["Paint me a fox", "Sketching"]);
	});

	test("a switch that read the old id lands on the new one", async () => {
		await saveTurn("bot:local_scribe", "Draft the memo");
		const switching = switchGlobalChatBot("local_scribe");
		await useGlobalChatStore
			.getState()
			.rebindBotChat("local_scribe", "bot_acct_scribe");
		await switching;
		expect(shown()).toEqual({
			conversationId: "bot:bot_acct_scribe",
			botId: "bot_acct_scribe",
			texts: ["Draft the memo"],
		});
		expect(
			useGlobalChatStore
				.getState()
				.messages.map((message) => message.sessionId),
		).toEqual(["bot:bot_acct_scribe"]);

		await restoreGlobalChatConversation("bot:local_scribe");
		await switchGlobalChatBot("local_scribe");
		expect(shown().conversationId).toBe("bot:bot_acct_scribe");
	});

	test("a switch to another bot during the move still lands", async () => {
		await saveTurn("bot:local_runner", "Plan the route");
		await switchGlobalChatBot("local_runner");
		const switching = switchGlobalChatBot("bot_writer");
		await useGlobalChatStore
			.getState()
			.rebindBotChat("local_runner", "bot_acct_runner");
		await switching;
		expect(shown().conversationId).toBe("bot:bot_writer");
	});

	test("writes addressed to the old id after the move land under the new one", async () => {
		await saveTurn("bot:local_poet", "Write a haiku");
		await switchGlobalChatBot("local_poet");
		await useGlobalChatStore
			.getState()
			.rebindBotChat("local_poet", "bot_acct_poet");

		const late = makeGlobalChatMessage(
			IRole.User,
			"About rain",
			"bot:local_poet",
		);
		useGlobalChatStore.getState().appendMessage(late);
		await persistGlobalChatMessage(late);
		await persistGlobalChatSession("bot:local_poet", "Poet");
		useGlobalChatStore.getState().enqueueMessage({
			conversationId: "bot:local_poet",
			content: "And snow",
		});
		const { reply, control, done } = liveTurn("bot:local_poet");
		setActiveRun("bot:local_poet", reply.id);

		const state = useGlobalChatStore.getState();
		expect(shown().texts).toEqual(["Write a haiku", "About rain"]);
		expect(state.messages.map((message) => message.sessionId)).toEqual([
			"bot:bot_acct_poet",
			"bot:bot_acct_poet",
		]);
		expect(state.queue.map((entry) => entry.conversationId)).toEqual([
			"bot:bot_acct_poet",
		]);
		expect(state.runs[reply.id]?.conversationId).toBe("bot:bot_acct_poet");
		expect(readActiveRuns().map((run) => run.conversationId)).toEqual([
			"bot:bot_acct_poet",
		]);
		expect((await globalChatDb.messages.get(late.id))?.sessionId).toBe(
			"bot:bot_acct_poet",
		);
		expect(await globalChatDb.sessions.get("bot:local_poet")).toBeUndefined();

		control.finish();
		await done;
		expect(
			await globalChatDb.messages
				.where("sessionId")
				.equals("bot:local_poet")
				.count(),
		).toBe(0);
	});

	test("is a no-op for a bot that never chatted", async () => {
		await saveTurn("flux-launch", "Plan the launch app");
		const before = useGlobalChatStore.getState();
		await before.rebindBotChat("local_quiet", "bot_acct_quiet");
		expect(useGlobalChatStore.getState()).toBe(before);
		expect(await globalChatDb.sessions.count()).toBe(1);
		expect(
			await globalChatDb.sessions.get("bot:bot_acct_quiet"),
		).toBeUndefined();
	});

	test("fills a chat that already shows the new id with the moved rows", async () => {
		await saveTurn("bot:local_helper", "Hello helper");
		await switchGlobalChatBot("bot_acct_helper");
		expect(shown().texts).toEqual([]);

		await useGlobalChatStore
			.getState()
			.rebindBotChat("local_helper", "bot_acct_helper");
		expect(shown()).toMatchObject({
			conversationId: "bot:bot_acct_helper",
			texts: ["Hello helper"],
		});
	});
});
