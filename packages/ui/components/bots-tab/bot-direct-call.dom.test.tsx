// @vitest-environment happy-dom

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { ConversationCallState } from "../../hooks/use-conversation-call";
import type { IBot } from "../../state/backend-state/bot-state";
import type { IConversation } from "../../state/backend-state/inbox-state";

const mocks = vi.hoisted(() => ({
	getConversation: vi.fn(),
	botConversation: vi.fn(),
	useConversationCall: vi.fn(),
	useConversationActivity: vi.fn(),
	useBotCallSpeech: vi.fn(),
	useSpeechModelState: vi.fn(),
	call: {} as ConversationCallState,
}));

const READY_SPEECH = {
	choice: { bitId: "stt", route: "remote" },
	settled: true,
	unsupported: false,
	pinnedUnusable: false,
};

vi.mock("../../state/backend-state", () => ({
	useBackend: () => ({
		inboxState: { getConversation: mocks.getConversation },
	}),
}));
vi.mock("../../state/global-chat/bot-history", () => ({
	botConversation: mocks.botConversation,
}));
vi.mock("../../hooks/use-conversation-call", () => ({
	useConversationCall: mocks.useConversationCall,
}));
vi.mock("../../hooks/use-conversation-activity", () => ({
	useConversationActivity: mocks.useConversationActivity,
}));
vi.mock("../../hooks/use-conversation-room", () => ({
	useConversationRoom: (conversation: IConversation) =>
		conversation.id ? { awareness: {}, provider: {} } : undefined,
}));
vi.mock("../../hooks/use-inbox", () => ({
	useInboxEntries: () => ({ data: { pages: [{ entries: [] }] } }),
}));
vi.mock("../../hooks/use-bot-call-speech", () => ({
	useBotCallSpeech: mocks.useBotCallSpeech,
}));
vi.mock("../../hooks/use-speech-model", () => ({
	useSpeechModelState: mocks.useSpeechModelState,
}));
vi.mock("./call-speech-setup", () => ({
	CallSpeechSetup: ({
		open,
		onOpenChange,
	}: {
		open: boolean;
		onOpenChange: (open: boolean) => void;
	}) => (
		<button
			type="button"
			data-open={open}
			aria-label="Set up calls"
			onClick={() => onOpenChange(true)}
		/>
	),
}));
vi.mock("./use-crew-directory", () => ({
	useCrewDirectory: () => (id: string) => ({
		id,
		name: id,
		isPending: false,
		isResolved: true,
	}),
}));
vi.mock("../identity/identity-avatar", () => ({
	IdentityAvatar: () => <span />,
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (_key: string, fallback: string, values?: Record<string, string>) =>
			fallback.replace(/\{\{(\w+)\}\}/g, (_, key) => values?.[key] ?? key),
	}),
}));

import { BotDirectCall } from "./bot-direct-call";

const bot = { id: "bot-1", owner_id: "owner-1", name: "Scout" } as IBot;
const conversation = {
	id: "direct-room",
	kind: "direct",
	members: [
		{ principal_id: "owner-1", kind: "user" },
		{ principal_id: "bot-1", kind: "bot" },
	],
} as IConversation;

let root: Root;
let host: HTMLDivElement;
let client: QueryClient;

beforeEach(() => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	vi.clearAllMocks();
	mocks.botConversation.mockResolvedValue(conversation.id);
	mocks.getConversation.mockResolvedValue(conversation);
	mocks.useSpeechModelState.mockReturnValue(READY_SPEECH);
	mocks.useConversationActivity.mockReturnValue({
		bots: [],
		typing: [],
		notifyTyping: vi.fn(),
		stopTyping: vi.fn(),
	});
	mocks.call = {
		available: false,
		live: null,
		joined: false,
		start: vi.fn(async () => {
			mocks.call.joined = true;
			mocks.call.live = { callId: "call-1", startedAt: null, participants: [] };
		}),
		join: vi.fn(async () => {
			mocks.call.joined = true;
		}),
		leave: vi.fn(async () => {
			mocks.call.joined = false;
		}),
		toggleMute: vi.fn(),
		listening: false,
		transcriptDraft: "",
		canListen: true,
		botsPresent: true,
		setListening: vi.fn((on: boolean) => {
			mocks.call.listening = on;
		}),
		setBotVoiceCapabilities: vi.fn(),
		canSpeakForBot: vi.fn(() => true),
		playBotSpeech: vi.fn(),
		stopBotSpeech: vi.fn(),
		speakingBot: null,
		interruptSpeech: vi.fn(),
		registerSpeechControl: vi.fn(),
		speechOutput: vi.fn(() => null),
		speechProgress: vi.fn(() => null),
		error: null,
		cameraOn: false,
		screenSharing: false,
		canUseCamera: false,
		canShareScreen: false,
		mediaPending: null,
		mediaError: null,
		toggleCamera: vi.fn(),
		toggleScreenShare: vi.fn(),
		videos: [],
	};
	mocks.useConversationCall.mockImplementation((_conversation, _me, room) => {
		mocks.call.available = Boolean(room);
		return mocks.call;
	});
	client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	client.clear();
	host.remove();
});

async function render(selected: IBot | undefined = bot) {
	await act(async () => {
		root.render(
			<QueryClientProvider client={client}>
				<BotDirectCall bot={selected}>
					{({ button, bar }) => (
						<>
							{button}
							{bar}
						</>
					)}
				</BotDirectCall>
			</QueryClientProvider>,
		);
		await new Promise((resolve) => setTimeout(resolve, 20));
	});
}

function button(label: string) {
	const found = [...host.querySelectorAll("button")].find(
		(node) =>
			node.getAttribute("aria-label") === label || node.textContent === label,
	);
	if (!found) throw new Error(`Missing button: ${label}`);
	return found;
}

test("the direct call is disabled until its existing bot conversation is ready", async () => {
	let resolve!: (conversation: IConversation) => void;
	mocks.getConversation.mockReturnValue(
		new Promise((done) => {
			resolve = done;
		}),
	);
	await render();
	expect(button("Start a call").disabled).toBe(true);
	await act(async () => {
		resolve(conversation);
	});
	await render();
	expect(button("Start a call").disabled).toBe(false);
	expect(mocks.botConversation).toHaveBeenCalledWith(expect.anything(), bot.id);
	expect(mocks.getConversation).toHaveBeenCalledWith(conversation.id);
});

test("starting a bot call listens automatically and addresses speech to that bot", async () => {
	await render();
	await render();
	await act(async () => button("Start a call").click());
	await render();
	expect(mocks.call.start).toHaveBeenCalledOnce();
	expect(mocks.call.setListening).toHaveBeenCalledExactlyOnceWith(true);
	const args = mocks.useConversationCall.mock.calls.at(-1);
	if (!args) throw new Error("The direct call hook did not run");
	expect(args[0].id).toBe(conversation.id);
	expect(args[1]).toBe(bot.owner_id);
	expect(args[4].mentionsIn("What is the weather?")).toEqual([bot.id]);
	expect(button("Leave call")).toBeDefined();
	expect(mocks.useBotCallSpeech).toHaveBeenLastCalledWith(
		expect.objectContaining({
			conversation,
			historyReady: true,
		}),
	);
});

test("joining through the live call bar also enables the bot's listening", async () => {
	mocks.call.live = { callId: "call-1", startedAt: null, participants: [] };
	await render();
	await render();
	const join = host.querySelector("section button") as HTMLButtonElement;
	expect(join.textContent).toBe("Join");
	await act(async () => join.click());
	await render();
	expect(mocks.call.join).toHaveBeenCalledOnce();
	expect(mocks.call.setListening).toHaveBeenCalledExactlyOnceWith(true);
});

test("a joined direct call shows the bot's queued and running work until leaving", async () => {
	mocks.call.live = { callId: "call-1", startedAt: null, participants: [] };
	mocks.useConversationActivity.mockReturnValue({
		bots: [{ botId: bot.id, status: "queued" }],
		typing: [],
	});
	await render();
	await render();
	expect(host.querySelector("[data-fl-conversation-activity]")).toBeNull();
	expect(mocks.useConversationActivity).not.toHaveBeenCalled();

	mocks.call.joined = true;
	await render();
	expect(host.textContent).toContain("Waiting for bot-1");
	expect(mocks.useConversationActivity).toHaveBeenLastCalledWith(
		conversation,
		bot.owner_id,
		undefined,
		expect.objectContaining({ awareness: {} }),
	);

	mocks.useConversationActivity.mockReturnValue({
		bots: [{ botId: bot.id, status: "running" }],
		typing: [],
	});
	await render();
	expect(host.textContent).toContain("bot-1 is answering");
	expect(host.textContent).not.toContain("Waiting for bot-1");

	await act(async () => button("Leave call").click());
	await render();
	expect(host.querySelector("[data-fl-conversation-activity]")).toBeNull();
});

test("a speech model arriving after join starts listening once and respects a later switch off", async () => {
	mocks.call.canListen = false;
	await render();
	await render();
	await act(async () => button("Start a call").click());
	await render();
	expect(mocks.call.setListening).not.toHaveBeenCalled();
	mocks.call.canListen = true;
	await render();
	expect(mocks.call.setListening).toHaveBeenCalledExactlyOnceWith(true);
	await render();
	await act(async () => {
		(host.querySelector("[role=switch]") as HTMLButtonElement).click();
	});
	await render();
	expect(mocks.call.setListening).toHaveBeenLastCalledWith(false);
	expect(mocks.call.setListening).toHaveBeenCalledTimes(2);
});

test("leaving before a speech model resolves clears the pending listening request", async () => {
	mocks.call.canListen = false;
	await render();
	await render();
	await act(async () => button("Start a call").click());
	await render();
	await act(async () => button("Leave call").click());
	await render();
	mocks.call.joined = true;
	mocks.call.canListen = true;
	await render();
	expect(mocks.call.setListening).not.toHaveBeenCalled();
});

test("a failed conversation lookup can be retried from the call control", async () => {
	mocks.getConversation.mockRejectedValueOnce(
		new Error("temporarily unavailable"),
	);
	await render();
	await render();
	expect(button("Retry call setup").disabled).toBe(false);
	await act(async () => button("Retry call setup").click());
	await render();
	expect(mocks.getConversation).toHaveBeenCalledTimes(2);
	expect(button("Start a call").disabled).toBe(false);
});

test("the call transcribes with the bot's own speech model", async () => {
	await render({ ...bot, stt_model: "hub:whisper" });
	expect(mocks.useSpeechModelState).toHaveBeenLastCalledWith(
		true,
		"hub:whisper",
	);
});

test("without a speech model the call control offers setup instead of a call", async () => {
	mocks.useSpeechModelState.mockReturnValue({
		...READY_SPEECH,
		choice: null,
	});
	await render();
	await render();
	expect(
		[...host.querySelectorAll("button")].some(
			(node) => node.getAttribute("aria-label") === "Start a call",
		),
	).toBe(false);
	const setup = button("Set up calls");
	await act(async () => setup.click());
	expect(button("Set up calls").dataset.open).toBe("true");
	expect(mocks.call.start).not.toHaveBeenCalled();
});

test("the setup stays open after a speech model resolves so the call can start from it", async () => {
	mocks.useSpeechModelState.mockReturnValue({
		...READY_SPEECH,
		choice: null,
	});
	await render();
	await render();
	await act(async () => button("Set up calls").click());
	mocks.useSpeechModelState.mockReturnValue(READY_SPEECH);
	await render();
	expect(button("Set up calls").dataset.open).toBe("true");
});

test("a speech model that is still loading leaves the call button as it was", async () => {
	mocks.useSpeechModelState.mockReturnValue({
		...READY_SPEECH,
		choice: null,
		settled: false,
	});
	await render();
	await render();
	expect(button("Start a call").disabled).toBe(false);
});

test("a failed call start does not leave a stale request to listen", async () => {
	mocks.call.start = vi.fn(async () => {
		mocks.call.error = "failed";
	});
	await render();
	await render();
	await act(async () => button("Start a call").click());
	await render();
	mocks.call.error = null;
	mocks.call.joined = true;
	await render();
	expect(mocks.call.setListening).not.toHaveBeenCalled();
});
