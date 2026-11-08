// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { useBackendStore } from "../state/backend-state";
import type { IBotTurn } from "../state/backend-state/bot-state";
import type {
	IConversation,
	IConversationTurn,
} from "../state/backend-state/inbox-state";
import { useRunningBotTurn } from "./use-bot-turn";
import { useConversationActivity } from "./use-conversation-activity";
import { inboxKeys } from "./use-inbox";

let root: Root;
let host: HTMLDivElement;
let client: QueryClient;
let listeners: Set<(turn: IBotTurn) => void>;
let serverTurns: IConversationTurn[];
const conversation = {
	id: "room",
	local: false,
	members: [{ kind: "bot", principal_id: "bot_atlas" }],
} as IConversation;

function NativeTurn() {
	useRunningBotTurn("bot_atlas");
	return null;
}

function RemoteTurn() {
	useConversationActivity(conversation, "owner", undefined, undefined);
	return null;
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	listeners = new Set();
	serverTurns = [];
	useBackendStore.getState().setBackend({
		botState: {
			turns: {
				list: async () => [],
				subscribe: async (listener: (turn: IBotTurn) => void) => {
					listeners.add(listener);
					return () => listeners.delete(listener);
				},
			},
		},
		inboxState: { listConversationTurns: async () => serverTurns },
	} as never);
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

test("desktop terminal turn events refresh bot settings once, while queued and waiting turns do not", async () => {
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<NativeTurn />
			</QueryClientProvider>,
		),
	);
	const invalidate = vi.spyOn(client, "invalidateQueries");
	const emit = async (status: IBotTurn["status"]) =>
		act(async () => {
			for (const listener of listeners)
				listener({
					id: "turn",
					bot_id: "bot_atlas",
					conversation_id: "room",
					status,
				});
		});
	await emit("queued");
	await emit("waiting");
	expect(invalidate).not.toHaveBeenCalled();
	await emit("done");
	expect(invalidate).toHaveBeenCalledWith({
		queryKey: ["bots", "detail", "bot_atlas"],
	});
	expect(invalidate).toHaveBeenCalledWith({ queryKey: ["bots", "list"] });
	const count = invalidate.mock.calls.length;
	await emit("done");
	expect(invalidate).toHaveBeenCalledTimes(count);
});

test("an existing room turn poll refreshes settings when its active bot finishes", async () => {
	serverTurns = [
		{
			id: "turn",
			bot_id: "bot_atlas",
			status: "running",
			created_at: new Date().toISOString(),
		} as IConversationTurn,
	];
	client.setQueryData(inboxKeys.turns("room"), serverTurns);
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<RemoteTurn />
			</QueryClientProvider>,
		),
	);
	const invalidate = vi.spyOn(client, "invalidateQueries");
	await act(async () => {
		serverTurns = [];
		client.setQueryData(inboxKeys.turns("room"), serverTurns);
		await new Promise((resolve) => setTimeout(resolve, 20));
	});
	expect(invalidate).toHaveBeenCalledWith({
		queryKey: ["bots", "detail", "bot_atlas"],
	});
	expect(invalidate).toHaveBeenCalledWith({ queryKey: ["bots", "list"] });
});
