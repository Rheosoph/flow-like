// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type {
	IConversation,
	IConversationTurn,
	IInboxEntry,
} from "../state/backend-state/inbox-state";
import { useConversationActivity } from "./use-conversation-activity";
import {
	inboxKeys,
	useSendInboxEntry,
	useSendMessageWithFiles,
} from "./use-inbox";

const backend = vi.hoisted(() => ({
	listConversationTurns: vi.fn(),
	sendEntry: vi.fn(),
}));
vi.mock("../state/backend-state", () => ({
	useBackend: () => ({ inboxState: backend }),
}));

const conversation = {
	id: "room",
	local: false,
	members: [{ kind: "bot", principal_id: "bot_atlas" }],
} as IConversation;
const turn: IConversationTurn = {
	id: "turn",
	bot_id: "bot_atlas",
	status: "queued",
	trigger_message_id: "message",
	created_at: "2026-11-08T12:00:00Z",
	updated_at: "2026-11-08T12:00:00Z",
};
const entry: IInboxEntry = {
	id: "entry",
	message_id: "message",
	conversation_id: "room",
	sender_id: "owner",
	kind: "message",
	envelope: { v: 1, enc: "none", body: { text: "Hello" } },
	created_at: turn.created_at,
	cursor: "2:entry",
};

let client: QueryClient;
let root: Root;
let host: HTMLDivElement;
let send: () => Promise<unknown>;

function Chat({ withFiles }: { withFiles: boolean }) {
	const activity = useConversationActivity(
		conversation,
		"owner",
		"1:previous-entry",
		undefined,
	);
	const plain = useSendInboxEntry();
	const files = useSendMessageWithFiles();
	send = () =>
		withFiles
			? files.mutateAsync({ conversationId: "room", body: { text: "Hello" } })
			: plain.mutateAsync({
					conversationId: "room",
					request: {
						message_id: entry.message_id,
						kind: entry.kind,
						envelope: entry.envelope,
					},
				});
	return <output>{activity.bots.map((bot) => bot.status).join(",")}</output>;
}

beforeEach(() => {
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	vi.resetAllMocks();
	backend.listConversationTurns.mockResolvedValue([]);
	backend.sendEntry.mockImplementation(async () => {
		backend.listConversationTurns.mockResolvedValue([turn]);
		return entry;
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
	vi.unstubAllGlobals();
});

test.each([false, true])(
	"sending shows bot activity before the next history poll (file composer: %s)",
	async (withFiles) => {
		await act(async () => {
			root.render(
				<QueryClientProvider client={client}>
					<Chat withFiles={withFiles} />
				</QueryClientProvider>,
			);
		});
		expect(client.getQueryData(inboxKeys.turns("room"))).toEqual([]);
		expect(host.textContent).toBe("");

		await act(async () => {
			await send();
			await new Promise((resolve) => setTimeout(resolve, 20));
		});

		expect(client.getQueryData(inboxKeys.turns("room"))).toEqual([turn]);
		expect(host.textContent).toBe("queued");
	},
);
