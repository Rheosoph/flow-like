import "fake-indexeddb/auto";
import {
	afterAll,
	afterEach,
	beforeEach,
	describe,
	expect,
	test,
} from "bun:test";
import { act, createElement } from "react";
import {
	domWindow,
	installDomGlobals,
	restoreDomGlobals,
	settle,
} from "./testing/dom-harness";

const { createRoot } = await import("react-dom/client");
const { QueryClient, QueryClientProvider } = await import(
	"@tanstack/react-query"
);
const { inboxKeys } = await import("../../hooks/use-inbox");
const { registerConversationFeed } = await import(
	"../../lib/realtime/conversation-feed"
);
const { useBackendStore } = await import("../../state/backend-state");
const { useBotHistoryStore } = await import(
	"../../state/global-chat/bot-history-store"
);
const { registerBotInbox, resolveBotTarget } = await import(
	"../../state/global-chat/bot-inbox"
);
const { useGlobalChatStore } = await import(
	"../../state/global-chat/global-chat-store"
);
const { useFollowBotChat } = await import(
	"../../state/global-chat/use-follow-bot-chat"
);
const { useBotChatRead } = await import("./use-bot-chat-read");
import type { ConversationFeedListener } from "../../lib/realtime/conversation-feed";
import type {
	IConversation,
	IInboxEntry,
	IInboxState,
} from "../../state/backend-state/inbox-state";

const BOT = "bot_read_test";
const CHAT = `bot:${BOT}`;
const CONVERSATION = "conversation_read_test";
const priorBackend = useBackendStore.getState().backend;
const priorChat = useGlobalChatStore.getState();
const priorHistory = useBotHistoryStore.getState();
let root: ReturnType<typeof createRoot>;
let container: HTMLElement;
let client: InstanceType<typeof QueryClient>;
let reads: Array<{ conversationId: string; upTo?: string }>;
let completeRead: (() => void) | undefined;
let holdRead = false;

function entry(index: number): IInboxEntry {
	return {
		id: `entry_${index}`,
		message_id: `message_${index}`,
		conversation_id: CONVERSATION,
		sender_id: BOT,
		kind: "message",
		envelope: { v: 1, enc: "none", body: { text: `Reply ${index}` } },
		created_at: new Date(index * 1000).toISOString(),
		cursor: `${index * 1_000_000}:entry_${index}`,
		seq: index,
	};
}

function conversation(index: number, unread = 1): IConversation {
	return {
		id: CONVERSATION,
		kind: "direct",
		title: null,
		encryption: "plain",
		created_by: "me",
		created_at: new Date(0).toISOString(),
		last_activity_at: entry(index).created_at,
		members: [],
		unread_count: unread,
		last_entry: entry(index),
	};
}

function cacheConversation(index: number, unread = 1) {
	const row = conversation(index, unread);
	client.setQueryData(inboxKeys.conversations(), [row]);
	client.setQueryData(inboxKeys.conversation(CONVERSATION), row);
}

function load(index: number, windowed = false) {
	useBotHistoryStore.getState().show(
		BOT,
		{
			messages: [],
			seqs: {},
			olderTo: null,
			newerFrom: null,
			newestCursor: entry(index).cursor,
		},
		windowed,
	);
}

function Probe({
	onScreen = true,
	botId = BOT,
}: { onScreen?: boolean; botId?: string | null }) {
	useFollowBotChat(CHAT);
	useBotChatRead(botId, onScreen);
	return null;
}

async function show(props: { onScreen?: boolean; botId?: string | null } = {}) {
	await act(async () =>
		root.render(
			createElement(
				QueryClientProvider,
				{ client },
				createElement(Probe, props),
			),
		),
	);
	await settle();
}

async function visibility(visible: boolean) {
	await act(async () => {
		Object.defineProperty(document, "visibilityState", {
			configurable: true,
			value: visible ? "visible" : "hidden",
		});
		document.dispatchEvent(new Event("visibilitychange"));
	});
	await settle();
}

beforeEach(async () => {
	installDomGlobals();
	Object.defineProperty(document, "visibilityState", {
		configurable: true,
		value: "visible",
	});
	reads = [];
	holdRead = false;
	completeRead = undefined;
	const inbox = {
		createConversation: async () => conversation(1),
		importHistory: async () => ({ imported: 0, skipped: 0 }),
		markRead: async (conversationId: string, upTo?: string) => {
			reads.push({ conversationId, upTo });
			if (holdRead)
				await new Promise<void>((resolve) => {
					completeRead = resolve;
				});
			return { last_read_at: entry(1).created_at };
		},
	} as unknown as IInboxState;
	useBackendStore.getState().setBackend({ inboxState: inbox } as never);
	registerBotInbox(inbox);
	await resolveBotTarget(BOT);
	useGlobalChatStore.setState({
		activeConversationId: CHAT,
		conversationLoading: false,
		messages: [],
	});
	useBotHistoryStore.setState({ views: {}, topics: {} });
	load(1);
	client = new QueryClient({
		defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
	});
	cacheConversation(1);
	container = document.createElement("div");
	document.body.appendChild(container);
	root = createRoot(container);
});

afterEach(async () => {
	completeRead?.();
	await act(async () => root.unmount());
	container.remove();
	client.clear();
	registerConversationFeed(null);
	registerBotInbox(null);
});

afterAll(() => {
	useBackendStore.setState({ backend: priorBackend });
	useGlobalChatStore.setState(priorChat);
	useBotHistoryStore.setState(priorHistory);
	Reflect.deleteProperty(domWindow.document, "visibilityState");
	restoreDomGlobals();
});

describe("a visible bot chat's read marker", () => {
	test("waits for restored history, clears both inbox caches and does not repeat the read", async () => {
		useGlobalChatStore.setState({ conversationLoading: true });
		await show();
		expect(reads).toEqual([]);
		await act(async () =>
			useGlobalChatStore.setState({ conversationLoading: false }),
		);
		await settle();
		expect(reads).toEqual([
			{ conversationId: CONVERSATION, upTo: entry(1).cursor },
		]);
		expect(
			client.getQueryData<IConversation[]>(inboxKeys.conversations())?.[0]
				.unread_count,
		).toBe(0);
		expect(
			client.getQueryData<IConversation>(inboxKeys.conversation(CONVERSATION))
				?.unread_count,
		).toBe(0);
		await show();
		expect(reads).toHaveLength(1);
	});

	test("waits while a sheet covers the chat or its document is hidden", async () => {
		await show({ onScreen: false });
		expect(reads).toEqual([]);
		await visibility(false);
		await show();
		expect(reads).toEqual([]);
		await visibility(true);
		expect(reads).toHaveLength(1);
	});

	test("does not acknowledge another bot or an older history window", async () => {
		await show({ botId: "bot_other" });
		expect(reads).toEqual([]);
		await act(async () => load(1, true));
		await show();
		expect(reads).toEqual([]);
		await act(async () => load(1));
		await settle();
		expect(reads).toHaveLength(1);
	});

	test("acknowledges incoming entries after the realtime transcript receives them", async () => {
		let receive: ConversationFeedListener | undefined;
		registerConversationFeed({
			watch: (_target, listener) => {
				receive = listener;
				return () => {};
			},
		});
		await show();
		await act(async () => {
			cacheConversation(2);
			receive?.([entry(2)]);
		});
		await settle();
		expect(useGlobalChatStore.getState().messages.at(-1)?.id).toBe(
			entry(2).message_id,
		);
		expect(reads.map((read) => read.upTo)).toEqual([
			entry(1).cursor,
			entry(2).cursor,
		]);
		expect(
			client.getQueryData<IConversation[]>(inboxKeys.conversations())?.[0]
				.unread_count,
		).toBe(0);
	});

	test("a late read response preserves messages that arrived after its cursor", async () => {
		holdRead = true;
		await show();
		expect(reads).toHaveLength(1);
		await act(async () => {
			cacheConversation(2);
			completeRead?.();
		});
		await settle();
		expect(
			client.getQueryData<IConversation[]>(inboxKeys.conversations())?.[0]
				.unread_count,
		).toBe(1);
		expect(
			client.getQueryData<IConversation>(inboxKeys.conversation(CONVERSATION))
				?.unread_count,
		).toBe(1);
	});
});
