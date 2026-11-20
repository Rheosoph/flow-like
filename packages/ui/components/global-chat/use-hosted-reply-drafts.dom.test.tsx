// @vitest-environment happy-dom
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import {
	deliverRealtimeReply,
	resetReplyStreams,
} from "../../lib/realtime/reply-streams";
import { IRole } from "../../lib/schema/llm/history";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import type { IMessage } from "../interfaces/chat-default/chat-db";
import { useHostedReplyDrafts } from "./use-hosted-reply-drafts";

vi.mock("../../state/global-chat/bot-inbox", () => ({
	activeBotTarget: () => ({ botId: "bot_atlas", conversationId: "dm_atlas" }),
}));

let root: Root;
let host: HTMLDivElement;
let seen: readonly IMessage[] = [];
const realMatchMedia = window.matchMedia;

function Probe({ botId }: Readonly<{ botId: string | null }>) {
	seen = useHostedReplyDrafts("bot:bot_atlas", botId);
	return null;
}

const reply = (text: string, state: "writing" | "done" = "writing") =>
	deliverRealtimeReply({
		protocol_version: 1,
		kind: "reply",
		conversation_id: "dm_atlas",
		message_id: "reply_1",
		reply_to: "question_1",
		sender_id: "bot_atlas",
		offset: 0,
		text,
		state,
	});

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	window.matchMedia = ((query: string) => ({
		matches: query.includes("reduced-motion"),
		media: query,
		addEventListener() {},
		removeEventListener() {},
	})) as unknown as typeof window.matchMedia;
	resetReplyStreams();
	useGlobalChatStore.setState({ messages: [] });
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
	seen = [];
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	window.matchMedia = realMatchMedia;
});

test("a reply the bot's own host writes shows as a live message until it is stored", async () => {
	await act(async () => root.render(<Probe botId="bot_atlas" />));
	expect(seen).toEqual([]);

	await act(async () => reply("Your flight leaves"));
	expect(seen.map((message) => [message.id, message.inner.content])).toEqual([
		["reply_1", "Your flight leaves"],
	]);
	expect(seen[0]?.inner.role).toBe(IRole.Assistant);

	await act(async () => reply("Your flight leaves at 09:40.", "done"));
	expect(seen[0]?.inner.content).toBe("Your flight leaves at 09:40.");

	await act(async () =>
		useGlobalChatStore.setState({
			messages: [{ ...(seen[0] as IMessage), timestamp: 1 }],
		}),
	);
	expect(seen).toEqual([]);
});

test("a chat that runs the bot itself shows no hosted drafts", async () => {
	await act(async () => root.render(<Probe botId={null} />));
	await act(async () => reply("Your flight leaves"));
	expect(seen).toEqual([]);
});
