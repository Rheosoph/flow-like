// @vitest-environment happy-dom
import "fake-indexeddb/auto";
import { beforeEach, describe, expect, test, vi } from "vitest";
import { IRole } from "../../lib/schema/llm/history";
import type {
	IConversationFileReady,
	IInboxEntry,
	IInboxImportEntry,
	IInboxState,
	ISendInboxEntryRequest,
} from "../backend-state/inbox-state";
import { useBotHistoryStore } from "./bot-history-store";
import {
	botHasInboxHistory,
	desktopInteractiveArgs,
	ensureBotTarget,
	jumpBotChat,
	registerBotInbox,
	sendInteractiveMessage,
	showLatestBotChat,
	startBotTopic,
	webInteractiveBody,
} from "./bot-inbox";
import { globalChatDb } from "./global-chat-db";
import { useGlobalChatStore } from "./global-chat-store";
import {
	makeGlobalChatMessage,
	persistGlobalChatMessage,
	switchGlobalChatBot,
} from "./global-chat-stream";

vi.mock("@tauri-apps/api/core", () => ({
	Channel: class {
		onmessage?: (chunk: string) => void;
	},
	invoke: vi.fn(async () => ({})),
}));

function inboxEntry(seq: number, sender: string, text: string): IInboxEntry {
	return {
		id: `e${seq}`,
		message_id: `m${seq}`,
		conversation_id: "conv_ada",
		sender_id: sender,
		kind: "message",
		envelope: { v: 1, enc: "none", body: { text } },
		created_at: new Date(Date.UTC(2026, 10, 1, 12, seq)).toISOString(),
		cursor: `${seq * 1000}:e${seq}`,
		seq,
	};
}

function backend(options: { failing?: boolean; refusesFiles?: boolean } = {}) {
	const stored = new Map<string, IInboxEntry>();
	const imported = new Map<string, IInboxImportEntry>();
	const sent: ISendInboxEntryRequest[] = [];
	const uploads: string[] = [];
	for (const [seq, sender, text] of [
		[1, "user_1", "Earlier question"],
		[2, "bot_ada", "Earlier answer"],
	] as const) {
		stored.set(`m${seq}`, inboxEntry(seq, sender, text));
	}
	const inbox = {
		async createConversation() {
			if (options.failing) throw new Error("no inbox for this bot");
			return { id: "conv_ada" };
		},
		async listEntries() {
			const entries = [...stored.values()].sort(
				(a, b) => (a.seq ?? 0) - (b.seq ?? 0),
			);
			return { entries, next_before: null, next_to_seq: null };
		},
		async importHistory(
			_bot: string,
			_conversation: string,
			batch: IInboxImportEntry[],
		) {
			let added = 0;
			for (const item of batch) {
				if (imported.has(item.message_id)) continue;
				imported.set(item.message_id, item);
				stored.set(
					item.message_id,
					inboxEntry(
						stored.size + 1,
						item.sender === "bot" ? "bot_ada" : "user_1",
						(item.envelope.body as { text: string }).text,
					),
				);
				added += 1;
			}
			return { imported: added, skipped: batch.length - added };
		},
		async sendEntry(_id: string, request: ISendInboxEntryRequest) {
			sent.push(request);
			return {
				...inboxEntry(stored.size + 1, "user_1", ""),
				message_id: request.message_id,
				kind: request.kind,
			};
		},
		async listSegments() {
			return { segments: [], next_after: null };
		},
		async uploadConversationFile(
			conversationId: string,
			file: File,
		): Promise<IConversationFileReady> {
			if (options.refusesFiles) {
				throw new Error("ATTACHMENTS_ENCRYPTED_UNSUPPORTED");
			}
			uploads.push(`${conversationId}:${file.name}`);
			const fileId = `f${uploads.length}`;
			return {
				file_id: fileId,
				name: file.name,
				app_id: `conversation_${conversationId}`,
				path: `${fileId}.png`,
				bytes: file.size,
				sha256: "a".repeat(64),
				mime: file.type,
			};
		},
	};
	return {
		inbox: inbox as unknown as IInboxState,
		sent,
		imported,
		stored,
		uploads,
	};
}

beforeEach(async () => {
	localStorage.clear();
	sessionStorage.clear();
	await globalChatDb.messages.clear();
	await globalChatDb.sessions.clear();
	useBotHistoryStore.setState({ views: {}, topics: {} });
	useGlobalChatStore.setState({
		activeConversationId: "flux-fresh",
		messages: [],
		runs: {},
		queue: [],
		draft: null,
		isStreaming: false,
		streamingMessages: [],
	});
	registerBotInbox(null);
});

function texts() {
	return useGlobalChatStore
		.getState()
		.messages.map((message) => message.inner.content);
}

describe("a bot chat on its inbox history", () => {
	test("opens with the inbox messages, and moves the old IndexedDB chat there first", async () => {
		const legacy = makeGlobalChatMessage(
			IRole.User,
			"From the old session",
			"bot:bot_ada",
		);
		await globalChatDb.messages.put(legacy);
		const { inbox, imported } = backend();
		registerBotInbox(inbox);

		await switchGlobalChatBot("bot_ada");

		expect([...imported.keys()]).toEqual([legacy.id]);
		expect(texts()).toEqual([
			"Earlier question",
			"Earlier answer",
			"From the old session",
		]);
		expect(botHasInboxHistory("bot_ada")).toBe(true);
	});

	test("stops writing the old sessions once the chat reads the inbox", async () => {
		registerBotInbox(backend().inbox);
		await switchGlobalChatBot("bot_ada");
		const message = makeGlobalChatMessage(IRole.User, "new", "bot:bot_ada");

		expect(await persistGlobalChatMessage(message)).toBe(true);
		expect(await globalChatDb.messages.get(message.id)).toBeUndefined();
	});

	test("a bot without an inbox keeps its saved sessions", async () => {
		const { inbox } = backend({ failing: true });
		registerBotInbox(inbox);
		const message = makeGlobalChatMessage(IRole.User, "kept", "bot:bot_ada");
		await globalChatDb.messages.put(message);

		await switchGlobalChatBot("bot_ada");

		expect(texts()).toEqual(["kept"]);
		expect(botHasInboxHistory("bot_ada")).toBe(false);
		expect(await persistGlobalChatMessage(message)).toBe(true);
		expect(await globalChatDb.messages.get(message.id)).toBeDefined();
	});

	test("Flux keeps its sessions even with an inbox", async () => {
		const { inbox, sent } = backend();
		registerBotInbox(inbox);
		const message = makeGlobalChatMessage(IRole.User, "flux", "flux-chat-1");
		expect(await persistGlobalChatMessage(message)).toBe(true);
		expect(await globalChatDb.messages.get(message.id)).toBeDefined();
		expect(sent).toEqual([]);
	});

	test("sending writes the message with the interactive mark and names it for the run", async () => {
		const { inbox, sent } = backend();
		registerBotInbox(inbox);
		await switchGlobalChatBot("bot_ada");

		const ref = await sendInteractiveMessage(
			"bot:bot_ada",
			{ id: "msg_1", text: "Hello again" },
			"run_1",
		);

		expect(ref).toEqual({
			conversationId: "conv_ada",
			triggerMessageId: "msg_1",
			clientRunId: "run_1",
		});
		expect(sent[0].envelope?.body).toEqual({
			text: "Hello again",
			interactive: { client_run_id: "run_1" },
		});
		expect(desktopInteractiveArgs(ref)).toEqual({ interactive: ref });
		expect(webInteractiveBody(ref)).toEqual({
			interactive: {
				conversation_id: "conv_ada",
				trigger_message_id: "msg_1",
				client_run_id: "run_1",
			},
		});
		expect(desktopInteractiveArgs(null)).toEqual({});

		await sendInteractiveMessage(
			"bot:bot_ada",
			{ id: "msg_2", text: "Spoken" },
			"run_2",
			{ call: "c", offset_ms: 0, duration_ms: 500, mentions: [] },
		);
		expect(sent[1].kind).toBe("transcript");
		expect(sent[1].envelope?.body).toEqual({
			call: "c",
			offset_ms: 0,
			duration_ms: 500,
			mentions: [],
			text: "Spoken",
			interactive: { client_run_id: "run_2" },
		});
		expect(webInteractiveBody(null)).toEqual({});
	});

	test("files go into the conversation first and travel with the message as attachments", async () => {
		const { inbox, sent, uploads } = backend();
		registerBotInbox(inbox);
		await switchGlobalChatBot("bot_ada");
		const logo = new File(["png"], "logo.png", { type: "image/png" });
		const huge = new File(["x"], "huge.mov", { type: "video/quicktime" });
		Object.defineProperty(huge, "size", { value: 64 * 1024 * 1024 });

		await sendInteractiveMessage(
			"bot:bot_ada",
			{ id: "m", text: "Our logo" },
			"r",
			undefined,
			[logo, huge],
		);

		expect(uploads).toEqual(["conv_ada:logo.png"]);
		expect(sent[0].envelope?.body).toEqual({
			text: "Our logo",
			interactive: { client_run_id: "r" },
			parts: [
				{
					kind: "attachment",
					id: "a1",
					name: "logo.png",
					handle: {
						app_id: "conversation_conv_ada",
						path: "f1.png",
						bytes: 3,
						sha256: "a".repeat(64),
						mime: "image/png",
					},
				},
			],
		});
	});

	test("a refused upload still sends the message, its files kept for the turn only", async () => {
		const { inbox, sent } = backend({ refusesFiles: true });
		registerBotInbox(inbox);
		vi.spyOn(console, "warn").mockImplementation(() => {});
		const logo = new File(["png"], "logo.png", { type: "image/png" });

		expect(
			await sendInteractiveMessage(
				"bot:bot_ada",
				{ id: "m", text: "x" },
				"r",
				undefined,
				[logo],
			),
		).toMatchObject({ triggerMessageId: "m" });
		expect(sent[0].envelope?.body).toEqual({
			text: "x",
			interactive: { client_run_id: "r" },
		});
	});

	test("a chat whose inbox nothing resolved yet resolves it before sending", async () => {
		const { inbox, sent } = backend();
		registerBotInbox(inbox);
		expect(
			await sendInteractiveMessage("bot:bot_ada", { id: "m", text: "x" }, "r"),
		).toMatchObject({ conversationId: "conv_ada", triggerMessageId: "m" });
		expect(sent[0].envelope?.body).toMatchObject({ text: "x" });
		expect(await ensureBotTarget("bot:bot_ada")).toEqual({
			botId: "bot_ada",
			conversationId: "conv_ada",
		});
	});

	test("a chat whose inbox can't be reached sends no mark", async () => {
		const { inbox, sent } = backend({ failing: true });
		registerBotInbox(inbox);
		vi.spyOn(console, "warn").mockImplementation(() => {});
		expect(
			await sendInteractiveMessage("bot:bot_ada", { id: "m", text: "x" }, "r"),
		).toBeNull();
		expect(sent).toEqual([]);
	});

	test("a failed inbox write lets the run go on without storing", async () => {
		const { inbox } = backend();
		registerBotInbox(inbox);
		await switchGlobalChatBot("bot_ada");
		vi.spyOn(inbox, "sendEntry").mockRejectedValueOnce(new Error("offline"));
		vi.spyOn(console, "warn").mockImplementation(() => {});
		expect(
			await sendInteractiveMessage("bot:bot_ada", { id: "m", text: "x" }, "r"),
		).toBeNull();
	});

	test("New topic writes a topic entry and remembers it for the jump list", async () => {
		const { inbox, sent } = backend();
		registerBotInbox(inbox);
		await switchGlobalChatBot("bot_ada");

		expect(await startBotTopic("bot_ada", "Holiday")).toBe(true);

		expect(sent[0].kind).toBe("topic");
		expect(sent[0].envelope?.body).toEqual({ title: "Holiday" });
		expect(useBotHistoryStore.getState().topics.bot_ada).toHaveLength(1);
	});

	test("a jump shows the window around the position and Back to latest returns", async () => {
		const { inbox } = backend();
		registerBotInbox(inbox);
		await switchGlobalChatBot("bot_ada");

		expect(await jumpBotChat("bot_ada", 1)).toBe(true);
		expect(useBotHistoryStore.getState().views.bot_ada.windowed).toBe(true);
		expect(await showLatestBotChat("bot_ada")).toBe(true);
		expect(useBotHistoryStore.getState().views.bot_ada.windowed).toBe(false);
	});

	test("a jump waits while a reply streams", async () => {
		registerBotInbox(backend().inbox);
		await switchGlobalChatBot("bot_ada");
		useGlobalChatStore.setState({ isStreaming: true });
		expect(await jumpBotChat("bot_ada", 1)).toBe(false);
	});
});
