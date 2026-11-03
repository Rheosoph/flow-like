// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { EmotionClient } from "../../lib/bot-emotion/client";
import { expressionScores } from "../../lib/bot-emotion/scoring";
import { IRole } from "../../lib/schema/llm/history";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import { CrewBotCard, type CrewMember } from "../bots-tab/crew-grid";
import { ChatBox } from "../interfaces/chat-default/chatbox";
import { GlobalChatBotAvatar, useDraftRelay } from "./chat-bot-avatar";
import { ChatBotReactionProvider } from "./chat-bot-reaction";

const inference = vi.hoisted(() => ({ predict: vi.fn() }));
vi.mock("../../lib/bot-emotion/client", () => ({
	getBotEmotionClient: (): EmotionClient => ({
		warmup() {},
		predict: inference.predict,
		cancel() {},
		subscribe(listener) {
			listener({ status: "ready" });
			return () => {};
		},
	}),
}));

const member = (key: string): CrewMember => ({
	key,
	selection: { type: "bot", botId: key },
	system: null,
	name: key,
	description: "Research assistant",
	encrypted: false,
	local: true,
	appearance: { kind: "pip" },
});
const crew = [member("research"), member("helper")];

function Surface({
	selected = "research",
	header = true,
}: { selected?: string; header?: boolean }) {
	const relay = useDraftRelay();
	return (
		<ChatBotReactionProvider>
			<div data-testid="crew">
				{crew.map((bot) => (
					<CrewBotCard
						key={bot.key}
						member={bot}
						selected={selected === bot.key}
						onSelect={() => {}}
					/>
				))}
			</div>
			{header && (
				<div data-testid="header">
					<GlobalChatBotAvatar
						relay={relay.listener}
						appearance={{ kind: "pip" }}
						size={48}
					/>
				</div>
			)}
			<ChatBox
				onSendMessage={async () => {}}
				onContentChange={relay.report}
				fileUpload={false}
				audioInput={false}
				availableTools={[]}
				defaultActiveTools={[]}
			/>
		</ChatBotReactionProvider>
	);
}

let root: Root;
let host: HTMLDivElement;
let previous: ReturnType<typeof useGlobalChatStore.getState>;

beforeEach(() => {
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	vi.stubGlobal("matchMedia", () => ({
		matches: true,
		addEventListener() {},
		removeEventListener() {},
	}));
	previous = useGlobalChatStore.getState();
	useGlobalChatStore.setState({
		activeConversationId: "bot:research",
		messages: [],
		toolPrompt: null,
		isStreaming: false,
	});
	inference.predict.mockReset();
	inference.predict.mockResolvedValue(expressionScores({ annoyance: 0.95 }));
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	useGlobalChatStore.setState(previous, true);
	vi.unstubAllGlobals();
});

function expressions() {
	return [...host.querySelectorAll('[data-testid="crew"] .fl-bot')].map((bot) =>
		bot.getAttribute("data-expression"),
	);
}

async function typeDraft(value: string) {
	const textarea = host.querySelector("textarea");
	if (!textarea) throw new Error("Missing composer");
	await act(async () => {
		Object.getOwnPropertyDescriptor(
			HTMLTextAreaElement.prototype,
			"value",
		)?.set?.call(textarea, value);
		textarea.dispatchEvent(new Event("input", { bubbles: true }));
	});
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 360));
	});
}

test("typed unsent text updates both real portraits with one inference and clears immediately", async () => {
	await act(async () => root.render(<Surface />));
	expect(expressions()).toEqual(["neutral", "neutral"]);
	await typeDraft("du kleiner wichser brings wirklich garnichts hin");
	expect(inference.predict).toHaveBeenCalledTimes(1);
	expect(inference.predict).toHaveBeenCalledWith(
		expect.anything(),
		expect.objectContaining({
			draft: "du kleiner wichser brings wirklich garnichts hin",
		}),
	);
	expect(
		host
			.querySelector('[data-testid="header"] .fl-bot')
			?.getAttribute("data-expression"),
	).toBe("concerned");
	expect(expressions()).toEqual(["concerned", "neutral"]);
	expect(
		host.querySelectorAll('[data-testid="crew"] [data-feature="brows"]').length,
	).toBeGreaterThan(0);
	await typeDraft("");
	expect(expressions()).toEqual(["neutral", "neutral"]);
	expect(inference.predict).toHaveBeenCalledTimes(1);
});

test("shares a sent wink with both portraits while keeping the working indicator", async () => {
	inference.predict.mockResolvedValue({ ...expressionScores({}), wink: 1 });
	await act(async () => root.render(<Surface />));
	await typeDraft("Thanks 😉");
	expect(expressions()).toEqual(["wink", "neutral"]);
	await act(async () =>
		useGlobalChatStore.setState({
			isStreaming: true,
			messages: [
				{
					id: "sent-wink",
					appId: "app",
					sessionId: "bot:research",
					timestamp: Date.now(),
					files: [],
					inner: { role: IRole.User, content: "Thanks 😉" },
				},
			],
		}),
	);
	await typeDraft("");
	expect(expressions()).toEqual(["wink", "neutral"]);
	expect(
		host
			.querySelector('[data-testid="header"] .fl-bot')
			?.getAttribute("data-expression"),
	).toBe("wink");
	expect(
		host
			.querySelector('[data-testid="crew"] .fl-bot')
			?.querySelectorAll("[data-loading-dot]"),
	).toHaveLength(3);
	inference.predict.mockResolvedValue(expressionScores({ annoyance: 0.95 }));
	await typeDraft("another draft");
	expect(expressions()[0]).not.toBe("wink");
});

test("does not lend a previous bot's reaction to a newly selected card and clears on close", async () => {
	await act(async () => root.render(<Surface />));
	await typeDraft("I am worried");
	expect(expressions()).toEqual(["concerned", "neutral"]);
	await act(async () => root.render(<Surface selected="helper" />));
	expect(expressions()).toEqual(["neutral", "neutral"]);
	await act(async () => root.render(<Surface header={false} />));
	expect(expressions()).toEqual(["neutral", "neutral"]);
});

test("keeps reacting while working and preserves the activity bubble", async () => {
	useGlobalChatStore.setState({ isStreaming: true });
	await act(async () => root.render(<Surface />));
	expect(expressions()).toEqual(["focused", "neutral"]);
	await typeDraft("I am worried");
	expect(expressions()).toEqual(["concerned", "neutral"]);
	const selected = host.querySelector('[data-testid="crew"] .fl-bot');
	expect(selected?.getAttribute("data-agent-state")).toBe("working");
	expect(selected?.querySelectorAll("[data-loading-dot]")).toHaveLength(3);
	await typeDraft("");
	expect(expressions()).toEqual(["focused", "neutral"]);
});
