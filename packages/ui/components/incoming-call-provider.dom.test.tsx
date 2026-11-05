// @vitest-environment happy-dom
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import type {
	IConversation,
	IInboxEntry,
} from "../state/backend-state/inbox-state";
import type { INotification } from "../state/backend-state/types";

const mocks = vi.hoisted(() => ({
	listNotifications: vi.fn(),
	getConversation: vi.fn(),
	listEntries: vi.fn(),
	listBlocks: vi.fn(),
	requestJoin: vi.fn(),
	clearJoin: vi.fn(),
	push: vi.fn(),
	watch: vi.fn(),
	states: new Map<number, unknown>(),
	listeners: new Set<() => void>(),
	roomAvailable: true,
	feed: undefined as undefined | ((entries: readonly IInboxEntry[]) => void),
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({
		t: (_key: string, fallback: string, options?: Record<string, string>) =>
			fallback.replace(/\{\{(\w+)\}\}/g, (_, key) => options?.[key] ?? ""),
	}),
}));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: mocks.push }) }));
vi.mock("../state/backend-state", () => {
	const backend = {
		userState: { listNotifications: mocks.listNotifications },
		inboxState: {
			getConversation: mocks.getConversation,
			listEntries: mocks.listEntries,
			listBlocks: mocks.listBlocks,
		},
	};
	return { useBackend: () => backend };
});
vi.mock("../lib/realtime/incoming-call-join", () => ({
	requestIncomingCallJoin: mocks.requestJoin,
	registerIncomingCallScope: (_backend: unknown, scope: string) => () =>
		mocks.clearJoin(scope),
}));
vi.mock("../lib/realtime/conversation-feed", () => ({
	watchConversation: (...args: unknown[]) => {
		mocks.watch(...args);
		mocks.feed = args[1] as typeof mocks.feed;
		return () => {
			mocks.feed = undefined;
		};
	},
}));
vi.mock("../hooks/use-conversation-room", () => {
	const room = {
		awareness: {
			clientID: 1,
			getStates: () => mocks.states,
			on: (_event: string, listener: () => void) =>
				mocks.listeners.add(listener),
			off: (_event: string, listener: () => void) =>
				mocks.listeners.delete(listener),
		},
	};
	return {
		useConversationRoom: () => (mocks.roomAvailable ? room : undefined),
	};
});
vi.mock("./identity/identity-avatar", () => ({
	IdentityAvatar: ({ subject }: { subject: string }) =>
		createElement("span", { "data-avatar": subject }),
}));

import { incomingCallResolutionKey } from "../lib/realtime/incoming-call-notification";
import { IncomingCallProvider } from "./incoming-call-provider";

let root: ReturnType<typeof createRoot>;
let client: QueryClient;
let scope: string;
let counter = 0;
let notice: INotification;
let conversation: IConversation;
let started: IInboxEntry;

beforeEach(() => {
	vi.clearAllMocks();
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	scope = `hub-${++counter}`;
	notice = {
		id: "notice",
		user_id: "me",
		title: "Anna",
		notification_type: "SYSTEM",
		read: false,
		link: "/bots?conversation=chat_1&call=call_1",
		created_at: new Date().toISOString(),
	};
	conversation = {
		id: "chat_1",
		kind: "direct",
		encryption: "plain",
		title: null,
		created_by: "anna",
		created_at: notice.created_at,
		last_activity_at: notice.created_at,
		unread_count: 1,
		last_entry: null,
		members: ["me", "anna"].map((id) => ({
			principal_id: id,
			kind: "user",
			added_by: null,
			joined_at: notice.created_at,
		})),
	};
	started = {
		id: "entry",
		message_id: "call_1",
		conversation_id: "chat_1",
		sender_id: "anna",
		kind: "call",
		envelope: { v: 1, enc: "none", body: { op: "started" } },
		cursor: "1",
		created_at: notice.created_at,
	};
	mocks.states.clear();
	mocks.listeners.clear();
	mocks.roomAvailable = true;
	mocks.states.set(2, {
		sub: "anna",
		call: { id: "call_1", joined: true, peer: "anna_peer" },
	});
	mocks.listNotifications.mockImplementation(async () => [notice]);
	mocks.getConversation.mockImplementation(async () => conversation);
	mocks.listEntries.mockImplementation(async () => ({
		entries: [started],
		next_before: null,
	}));
	mocks.listBlocks.mockResolvedValue([]);
	mocks.feed = undefined;
	localStorage.clear();
	client = new QueryClient({
		defaultOptions: { queries: { retry: false, gcTime: 0 } },
	});
	const container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await act(async () => root.unmount());
	client.clear();
	document.body.innerHTML = "";
	vi.unstubAllGlobals();
	vi.restoreAllMocks();
});

async function render(props = {}) {
	await act(async () => {
		root.render(
			createElement(
				QueryClientProvider,
				{ client },
				createElement(IncomingCallProvider, { scope, userId: "me", ...props }),
			),
		);
	});
	await settle();
}
async function settle() {
	for (let index = 0; index < 5; index++) {
		await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
	}
}
function button(label: string) {
	const found = [...document.querySelectorAll("button")].find(
		(button) => button.textContent === label,
	);
	if (!found) throw new Error(`Missing ${label} button`);
	return found;
}

describe("incoming calls across the app", () => {
	test("shows the live caller and waits for Join before requesting media or navigating", async () => {
		await render();
		expect(
			document.querySelector('[role="alertdialog"]')?.textContent,
		).toContain("Incoming call");
		expect(document.querySelector('[data-avatar="anna"]')).not.toBeNull();
		expect(mocks.requestJoin).not.toHaveBeenCalled();
		expect(mocks.push).not.toHaveBeenCalled();
		await act(async () => button("Join").click());
		expect(mocks.requestJoin).toHaveBeenCalledWith({
			scope,
			userId: "me",
			conversationId: "chat_1",
			callId: "call_1",
		});
		expect(mocks.push).toHaveBeenCalledWith("/bots?conversation=chat_1");
		expect(document.querySelector('[role="alertdialog"]')).toBeNull();
		expect(mocks.clearJoin).not.toHaveBeenCalled();
	});

	test("Dismiss removes the prompt without joining and does not replay on refetch", async () => {
		vi.spyOn(Date, "now").mockReturnValue(Date.now());
		await render();
		await act(async () => button("Dismiss").click());
		await act(async () => client.invalidateQueries());
		await settle();
		expect(document.querySelector('[role="alertdialog"]')).toBeNull();
		expect(mocks.requestJoin).not.toHaveBeenCalled();
		expect(mocks.push).not.toHaveBeenCalled();
	});

	test("retries a transient validation failure without resolving the call in other tabs", async () => {
		mocks.getConversation.mockRejectedValueOnce(
			new Error("temporarily offline"),
		);
		await render();
		expect(document.querySelector('[role="alertdialog"]')).toBeNull();
		expect(
			localStorage.getItem(incomingCallResolutionKey(scope, "me")),
		).toBeNull();
		await act(async () =>
			client.invalidateQueries({ queryKey: ["inbox", "incoming-call"] }),
		);
		await settle();
		expect(document.querySelector('[role="alertdialog"]')).not.toBeNull();
	});

	test("room loss hides locally and can recover without resolving other tabs", async () => {
		await render();
		mocks.roomAvailable = false;
		await render();
		expect(document.querySelector('[role="alertdialog"]')).toBeNull();
		expect(
			localStorage.getItem(incomingCallResolutionKey(scope, "me")),
		).toBeNull();
		mocks.roomAvailable = true;
		await render();
		expect(document.querySelector('[role="alertdialog"]')).not.toBeNull();
	});

	test("missing peer presence hides locally and can recover", async () => {
		await render();
		const states = new Map(mocks.states);
		mocks.states.clear();
		await act(async () => {
			for (const listener of mocks.listeners) listener();
		});
		expect(document.querySelector('[role="alertdialog"]')).toBeNull();
		expect(
			localStorage.getItem(incomingCallResolutionKey(scope, "me")),
		).toBeNull();
		for (const [id, state] of states) mocks.states.set(id, state);
		await act(async () => {
			for (const listener of mocks.listeners) listener();
		});
		expect(document.querySelector('[role="alertdialog"]')).not.toBeNull();
	});

	test("does not prompt for a blocked caller", async () => {
		mocks.listBlocks.mockResolvedValue([
			{ principal_id: "anna", kind: "user", created_at: notice.created_at },
		]);
		await render();
		expect(document.querySelector('[role="alertdialog"]')).toBeNull();
	});

	test.each([
		"stale",
		"read",
		"other-account",
		"ordinary-message",
		"external-link",
	])("ignores %s notifications", async (kind) => {
		if (kind === "stale")
			notice.created_at = new Date(Date.now() - 91_000).toISOString();
		if (kind === "read") notice.read = true;
		if (kind === "other-account") notice.user_id = "someone-else";
		if (kind === "ordinary-message") notice.link = "/bots?conversation=chat_1";
		if (kind === "external-link")
			notice.link = "https://example.com/bots?conversation=chat_1&call=call_1";
		await render();
		expect(document.querySelector('[role="alertdialog"]')).toBeNull();
		expect(mocks.getConversation).not.toHaveBeenCalled();
	});

	test.each(["muted", "local", "encrypted", "self", "missing-start", "ended"])(
		"does not prompt for a %s call",
		async (kind) => {
			if (kind === "muted") conversation.muted = true;
			if (kind === "local") conversation.local = true;
			if (kind === "encrypted") conversation.encryption = "e2e";
			if (kind === "self") started.sender_id = "me";
			if (kind === "missing-start") started.message_id = "other_call";
			if (kind === "ended")
				mocks.listEntries.mockResolvedValue({
					entries: [ended(), started],
					next_before: null,
				});
			await render();
			expect(document.querySelector('[role="alertdialog"]')).toBeNull();
		},
	);

	test("waits for matching live presence and ignores a call joined in another tab", async () => {
		mocks.states.clear();
		await render();
		expect(document.querySelector('[role="alertdialog"]')).toBeNull();
		mocks.states.set(2, {
			sub: "anna",
			call: { id: "call_1", joined: true, peer: "anna_peer" },
		});
		await act(async () => {
			for (const listener of mocks.listeners) listener();
		});
		expect(document.querySelector('[role="alertdialog"]')).not.toBeNull();
		mocks.states.set(3, {
			sub: "me",
			call: { id: "call_1", joined: true, peer: "other_tab" },
		});
		await act(async () => {
			for (const listener of mocks.listeners) listener();
		});
		expect(document.querySelector('[role="alertdialog"]')).toBeNull();
	});

	test("an end event closes the prompt without waiting for the notification poll", async () => {
		await render();
		await act(async () => mocks.feed?.([ended()]));
		expect(document.querySelector('[role="alertdialog"]')).toBeNull();
	});

	test("a resolution in another tab closes the prompt", async () => {
		await render();
		const key = incomingCallResolutionKey(scope, "me");
		const value = JSON.stringify({ call_1: Date.now() + 90_000 });
		await act(async () => {
			localStorage.setItem(key, value);
			window.dispatchEvent(
				new StorageEvent("storage", { key, newValue: value }),
			);
		});
		expect(document.querySelector('[role="alertdialog"]')).toBeNull();
	});

	test("switching accounts clears the old join intent and prompt", async () => {
		await render();
		await render({ userId: "other" });
		expect(mocks.clearJoin).toHaveBeenCalledWith(scope);
		expect(document.querySelector('[role="alertdialog"]')).toBeNull();
	});
});

function ended(): IInboxEntry {
	return {
		...started,
		id: "ended",
		message_id: "end_1",
		cursor: "2",
		envelope: {
			v: 1,
			enc: "none",
			body: { op: "ended", call: "call_1", duration_ms: 1000 },
		},
	};
}
