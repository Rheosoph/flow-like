// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { IRole } from "../../lib/schema/llm/history";
import type { IConversation } from "../../state/backend-state/inbox-state";
import { useGlobalChatStore } from "../../state/global-chat/global-chat-store";
import type { BotAvatarProps } from "../bots/types";
import { BotsNavAvatar } from "./bots-nav-avatar";

const source = vi.hoisted(() => ({
	conversations: [] as IConversation[],
	history: null as { botId: string | null; at: number } | null,
	lookup: vi.fn(),
	signedIn: true,
}));

vi.mock("dexie-react-hooks", () => ({ useLiveQuery: () => source.history }));
vi.mock("react-oidc-context", () => ({
	useAuth: () => ({ user: { profile: { sub: "me" } } }),
}));
vi.mock("../../state/backend-state", () => ({
	useSignedIn: () => source.signedIn,
}));
vi.mock("../../hooks/use-inbox", () => ({
	useConversations: () => ({ data: source.conversations }),
}));
vi.mock("../../hooks/use-user-lookup", () => ({
	useUserLookup: (id?: string | null) => {
		source.lookup(id);
		return {
			data:
				id === "shared" ? { bot: { appearance: { kind: "cat" } } } : undefined,
		};
	},
}));
vi.mock("./use-global-chat-bot", () => ({
	useChatBotView: (id: string | null) => ({
		bot: id === "shared" ? undefined : { id },
		appearance: { kind: id === null ? "flux" : id },
	}),
}));
vi.mock("../bots/bot-avatar", () => ({
	BotAvatar: ({
		appearance,
		size,
		animated,
		decorative,
		agentState,
	}: BotAvatarProps) => (
		<span
			data-kind={appearance?.kind}
			data-size={size}
			data-animated={animated}
			data-decorative={decorative}
			data-state={agentState}
		/>
	),
}));

let host: HTMLDivElement;
let root: Root;
beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	source.conversations = [];
	source.history = null;
	source.signedIn = true;
	source.lookup.mockClear();
	useGlobalChatStore.setState({ messages: [], runs: {}, toolPrompt: null });
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});
afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
});

const kind = () => host.firstElementChild?.getAttribute("data-kind");
const render = () => act(async () => root.render(<BotsNavAvatar size={36} />));

function conversation(botId: string, timestamp: number): IConversation {
	return {
		id: botId,
		kind: "direct",
		title: null,
		encryption: "plain",
		created_by: "me",
		created_at: new Date(timestamp).toISOString(),
		last_activity_at: new Date(timestamp).toISOString(),
		members: [
			{ principal_id: "me", kind: "user", added_by: null, joined_at: "" },
			{ principal_id: botId, kind: "bot", added_by: null, joined_at: "" },
		],
		unread_count: 1,
		last_entry: {
			id: botId,
			message_id: botId,
			conversation_id: botId,
			sender_id: botId,
			kind: "message",
			envelope: { v: 1, enc: "none", body: { text: "Hello" } },
			created_at: new Date(timestamp).toISOString(),
			cursor: "1",
		},
	};
}

test("shows an animated, decorative Assistant before any conversations", async () => {
	await render();
	expect(kind()).toBe("general");
	expect(host.firstElementChild?.getAttribute("data-animated")).toBe("true");
	expect(host.firstElementChild?.getAttribute("data-decorative")).toBe("true");
	expect(host.firstElementChild?.getAttribute("data-size")).toBe("36");
});

test("follows incoming replies and retains the bot when its unread count clears", async () => {
	source.history = { botId: null, at: 1000 };
	await render();
	expect(kind()).toBe("flux");
	source.conversations = [conversation("scout", 2000)];
	await render();
	expect(kind()).toBe("scout");
	source.conversations[0].unread_count = 0;
	await render();
	expect(kind()).toBe("scout");
});

test("updates immediately on send and stays with that bot when another empty chat opens", async () => {
	source.conversations = [conversation("scout", 2000)];
	await render();
	await act(async () =>
		useGlobalChatStore.setState({
			messages: [
				{
					id: "sent",
					appId: "global",
					sessionId: "bot:helper",
					timestamp: 3000,
					inner: { role: IRole.User, content: "Hello" },
					files: [],
				},
			],
		}),
	);
	expect(kind()).toBe("helper");
	await act(async () =>
		useGlobalChatStore.setState({
			messages: [],
			activeConversationId: "bot:empty",
		}),
	);
	expect(kind()).toBe("helper");
});

test("uses the selected shared bot's directory appearance and avoids signed-out lookups", async () => {
	source.conversations = [conversation("shared", 2000)];
	await render();
	expect(kind()).toBe("cat");
	expect(source.lookup).toHaveBeenLastCalledWith("shared");
	source.signedIn = false;
	await render();
	expect(source.lookup).toHaveBeenLastCalledWith(undefined);
});
