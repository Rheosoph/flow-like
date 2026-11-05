// @vitest-environment happy-dom

import "fake-indexeddb/auto";
import { type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { ExecutionEngineProvider } from "../../lib/execution-engine";
import type { IIntercomEvent } from "../../lib/schema/events/intercom-event";
import type { IBackendState, IEventState } from "../../state/backend-state";
import { ChatInterface } from "./chat-default";
import {
	type IChatMainWidget,
	type IMessage,
	chatDb,
	decodeMessageRow,
	latestChatDraft,
	putChatDrafts,
	putChatMessage,
} from "./chat-default/chat-db";
import { resolveMainChatWidget } from "./chat-default/main-widget-state";
import type { IUseInterfaceProps } from "./interfaces";

const state = vi.hoisted(() => ({
	engine: undefined as unknown as ExecutionEngineProvider,
	backend: {} as IBackendState,
	router: { push: vi.fn(), replace: vi.fn() },
	params: new URLSearchParams({ sessionId: "session" }),
	setQueryParams: vi.fn(),
}));

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("next/navigation", () => ({ usePathname: () => "/use" }));
vi.mock("../../lib", async () => ({
	...(await import("../../lib/schema/llm/history")),
	...(await import("../../lib/llm/response")),
}));
vi.mock("../../lib/client-navigation", () => ({
	useClientRouter: () => state.router,
}));
vi.mock("../../lib/set-query-params", () => ({
	useClientSearchParams: () => state.params,
	useSetQueryParams: () => state.setQueryParams,
}));
vi.mock("../../lib/telemetry/errors", () => ({
	captureTelemetryError: vi.fn(),
}));
vi.mock("../../lib/widget-snapshot", () => ({
	captureWidgetSnapshots: vi.fn(),
}));
vi.mock("../../state/backend-state", () => ({
	useBackend: () => state.backend,
}));
vi.mock("../../state/execution-engine-context", () => ({
	useExecutionEngine: () => state.engine,
}));
vi.mock("../a2ui/frontend-state", () => ({ getFrontendStateStore: vi.fn() }));
vi.mock("../a2ui/workflow-payload", () => ({
	buildWorkflowFrontendContext: vi.fn(),
}));
vi.mock("../ui", () => {
	const Empty = () => null;
	return Object.fromEntries(
		[
			"AlertDialog",
			"AlertDialogAction",
			"AlertDialogCancel",
			"AlertDialogContent",
			"AlertDialogDescription",
			"AlertDialogFooter",
			"AlertDialogHeader",
			"AlertDialogTitle",
			"Button",
			"DropdownMenu",
			"DropdownMenuContent",
			"DropdownMenuItem",
			"DropdownMenuTrigger",
			"HoverCard",
			"HoverCardContent",
			"HoverCardTrigger",
		].map((name) => [name, Empty]),
	);
});
vi.mock("./chat-default/appearance", () => ({
	ChatAppearance: ({ children }: { children: ReactNode }) => children,
}));
vi.mock("./chat-default/chat-widget-execution", () => ({
	ChatWidgetExecutionProvider: ({ children }: { children: ReactNode }) =>
		children,
}));
vi.mock("./chat-default/chat", () => ({
	Chat: ({ messages }: { messages: IMessage[] }) => (
		<div>
			{messages.map((message) => (
				<div key={message.id}>{message.error?.message}</div>
			))}
		</div>
	),
}));
vi.mock("./chat-default/chat-db", async (importOriginal) => {
	const actual =
		await importOriginal<typeof import("./chat-default/chat-db")>();
	return { ...actual, latestChatDraft: vi.fn(actual.latestChatDraft) };
});
vi.mock("./chat-default/attachment", () => ({ fileToAttachment: vi.fn() }));
vi.mock("./chat-default/history", () => ({ ChatHistory: () => null }));
vi.mock("./chat-default/welcome", () => ({ ChatWelcome: () => null }));
vi.mock("./chat-default/respond-interaction", () => ({
	submitInteractionResponse: vi.fn(),
}));
vi.mock("./chat-default/use-chat-model", () => ({
	useChatModel: () => ({ selectedModel: undefined }),
}));

const failureMessage =
	"The active profile changed before automation could start. Run the workflow again.";
const event = {
	id: "chat",
	name: "Support",
	board_id: "board",
} as IUseInterfaceProps["event"];
let root: Root;
let container: HTMLDivElement;

beforeEach(async () => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	state.engine = new ExecutionEngineProvider();
	vi.mocked(latestChatDraft).mockClear();
	await chatDb.messages.clear();
	await chatDb.drafts.clear();
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
	const report = console.error;
	vi.spyOn(console, "error").mockImplementation((...args) => {
		if (args[0] !== "Execution error:") report(...args);
	});
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.restoreAllMocks();
});

async function waitForChat(assertion: () => void | Promise<void>) {
	const deadline = Date.now() + 1_000;
	while (true) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 10));
		});
		try {
			await assertion();
			return;
		} catch (error) {
			if (Date.now() >= deadline) throw error;
		}
	}
}

async function failDetachedRun(
	failure: unknown,
	events: IIntercomEvent[] = [],
) {
	const eventState: Pick<IEventState, "executeEvent"> = {
		executeEvent: async (_app, _event, _payload, _stream, _start, onEvents) => {
			onEvents?.(events);
			throw failure;
		},
	};
	state.engine.setBackend({ eventState } as IBackendState);
	await expect(
		state.engine.executeEvent("session", {
			appId: "app",
			eventId: "chat",
			payload: { id: "node" },
		}),
	).rejects.toBe(failure);
	expect(state.engine.isStreamComplete("session")).toBe(true);
}

async function expectPersistedFailure() {
	await act(async () =>
		root.render(<ChatInterface appId="app" event={event} />),
	);
	await waitForChat(async () => {
		const messages = (await chatDb.messages.toArray()).map(decodeMessageRow);
		const replies = messages.filter(
			(message) => message.inner.role === "assistant",
		);
		expect(replies, container.textContent || "empty DOM").toHaveLength(1);
		expect(replies[0].error).toMatchObject({
			title: "Support could not finish this response",
			message: failureMessage,
		});
	});
	await waitForChat(() => {
		expect(container.textContent).toContain(failureMessage);
	});
	expect(state.engine.hasStream("session")).toBe(false);
}

async function putUserMessage() {
	await putChatMessage({
		id: "user",
		appId: "app",
		sessionId: "session",
		timestamp: 0,
		files: [],
		inner: { role: "user", content: "Start automation" },
	} as IMessage);
}

async function completeDetachedRun(events: IIntercomEvent[]) {
	const eventState: Pick<IEventState, "executeEvent"> = {
		executeEvent: async (_app, _event, _payload, _stream, _start, onEvents) => {
			onEvents?.(events);
			return undefined;
		},
	};
	state.engine.setBackend({ eventState } as IBackendState);
	await state.engine.executeEvent("session", {
		appId: "app",
		eventId: "chat",
		payload: { id: "node" },
	});
	await waitForChat(() => {
		expect(state.engine.isStreamComplete("session")).toBe(true);
	});
}

async function putMainWidgetHistory() {
	const main = {
		command_id: "show-browser",
		title: "Browser",
		display: "open",
		widget: {
			instance_id: "browser",
			widget_id: "browser-widget",
			surface_id: "browser",
			component: { type: "text", text: { literalString: "Starting" } },
			updates: [],
		},
	} satisfies IChatMainWidget;
	await putChatMessage({
		id: "prior-turn",
		appId: "app",
		sessionId: "session",
		timestamp: 1,
		files: [],
		inner: { role: "assistant", content: "Opened the browser" },
		main_widget: main,
	} as IMessage);
	return main;
}

describe("chat main widget reconnect", () => {
	test("inherits the prior pane and preserves replayed updates across response resends and reopening", async () => {
		await putUserMessage();
		const main = await putMainWidgetHistory();
		const updates = ["Loading page", "Page loaded"].map((text) => ({
			type: "upsertElement",
			element_id: "browser",
			value: { type: "setText", text },
		}));
		await putChatDrafts([
			{
				id: "reply:backend-second",
				stream_root_id: "reply",
				stream_root_timestamp: 10,
				stream_message_id: "backend-second",
				appId: "app",
				sessionId: "session",
				timestamp: 11,
				files: [],
				inner: { role: "assistant", content: "Finished" },
				main_widget: {
					...main,
					widget: { ...main.widget, updates },
				},
			} as IMessage,
		]);
		const response = (content: string) => ({
			choices: [{ index: 0, message: { role: "assistant", content } }],
		});
		await completeDetachedRun([
			{
				event_id: "first-response",
				event_type: "chat_stream",
				payload: {
					message_id: "backend-first",
					response: response("Opening page"),
				},
			},
			{ event_id: "loading", event_type: "a2ui", payload: updates[0] },
			{
				event_id: "second-response",
				event_type: "chat_stream",
				payload: {
					message_id: "backend-second",
					response: response("Finished"),
					main_widget: main,
				},
			},
			{ event_id: "loaded", event_type: "a2ui", payload: updates[1] },
			{
				event_id: "final-response",
				event_type: "chat_out",
				payload: {
					message_id: "backend-second",
					response: response("Finished"),
					main_widget: main,
				},
			},
		] as IIntercomEvent[]);
		await act(async () =>
			root.render(<ChatInterface appId="app" event={event} />),
		);
		await waitForChat(async () => {
			const messages = (await chatDb.messages.toArray()).map(decodeMessageRow);
			const first = messages.find((message) => message.id === "reply");
			const second = messages.find(
				(message) => message.id === "reply:backend-second",
			);
			expect(first?.main_widget?.widget?.updates).toEqual([updates[0]]);
			expect(second?.main_widget?.widget?.updates).toEqual(updates);
			expect(second?.error).toBeUndefined();
			expect(
				messages.find((message) => message.id === "prior-turn")?.main_widget
					?.widget?.updates,
			).toEqual([]);
			expect(await chatDb.drafts.count()).toBe(0);
			expect(state.engine.hasStream("session")).toBe(false);
		});
		await act(async () => root.unmount());
		root = createRoot(container);
		await act(async () =>
			root.render(<ChatInterface appId="app" event={event} />),
		);
		await waitForChat(async () => {
			const messages = (await chatDb.messages.toArray()).map(decodeMessageRow);
			expect(
				resolveMainChatWidget(messages, "session")?.widget?.updates,
			).toEqual(updates);
			expect(messages).toHaveLength(4);
		});
	});

	test("restores an explicit clear after reopening without reviving the earlier pane", async () => {
		await putUserMessage();
		await putMainWidgetHistory();
		await completeDetachedRun([
			{
				event_id: "clear-pane",
				event_type: "chat_stream_partial",
				payload: {
					message_id: "clear-response",
					main_widget: { command_id: "clear-browser", widget: null, title: "" },
				},
			},
			{
				event_id: "final-response",
				event_type: "chat_out",
				payload: { message_id: "clear-response" },
			},
		] as IIntercomEvent[]);
		await act(async () =>
			root.render(<ChatInterface appId="app" event={event} />),
		);
		await waitForChat(async () => {
			const messages = (await chatDb.messages.toArray()).map(decodeMessageRow);
			expect(resolveMainChatWidget(messages, "session")).toMatchObject({
				command_id: "clear-browser",
				widget: null,
			});
			expect(state.engine.hasStream("session")).toBe(false);
		});
		await act(async () => root.unmount());
		root = createRoot(container);
		await act(async () =>
			root.render(<ChatInterface appId="app" event={event} />),
		);
		await waitForChat(async () => {
			const messages = (await chatDb.messages.toArray()).map(decodeMessageRow);
			expect(resolveMainChatWidget(messages, "session")?.widget).toBeNull();
			expect(messages).toHaveLength(3);
		});
	});
});

describe("chat reconnect after a failed run", () => {
	test("replays every response under its original IDs after a multi-message checkpoint", async () => {
		await putUserMessage();
		const messages = ["First answer", "Second answer"].map(
			(content, index) => ({
				id: index === 0 ? "reply" : "reply:backend-second",
				stream_root_id: "reply",
				stream_root_timestamp: 10,
				stream_message_id: index === 0 ? "backend-first" : "backend-second",
				appId: "app",
				sessionId: "session",
				timestamp: 10 + index,
				files: [],
				inner: { role: "assistant", content },
			}),
		) as IMessage[];
		await putChatDrafts(messages);
		const events = messages.map((message, index) => ({
			event_id: `response-${index}`,
			event_type: "chat_stream",
			payload: {
				message_id: message.stream_message_id,
				response: {
					choices: [{ index: 0, message: message.inner }],
				},
				attachments: [],
				widgets: [],
			},
		})) as IIntercomEvent[];
		await failDetachedRun(failureMessage, events);
		await act(async () =>
			root.render(<ChatInterface appId="app" event={event} />),
		);
		await waitForChat(async () => {
			const replies = (await chatDb.messages.toArray())
				.map(decodeMessageRow)
				.filter((message) => message.inner.role === "assistant")
				.sort((a, b) => a.timestamp - b.timestamp);
			expect(
				replies.map((message) => [message.id, message.inner.content]),
			).toEqual([
				["reply", "First answer"],
				["reply:backend-second", "Second answer"],
			]);
			expect(replies[0].error).toBeUndefined();
			expect(replies[1].error?.message).toBe(failureMessage);
			expect(await chatDb.drafts.count()).toBe(0);
		});
	});

	test.each([failureMessage, { error: failureMessage }])(
		"preserves a native preamble rejection without any streamed events: %j",
		async (failure) => {
			await failDetachedRun(failure);
			await expectPersistedFailure();
		},
	);

	test("preserves the failure when the run had already announced its ID", async () => {
		await putUserMessage();
		await failDetachedRun({ error: failureMessage }, [
			{
				event_id: "started",
				event_type: "run_initiated",
				payload: { run_id: "run" },
			},
		] as IIntercomEvent[]);
		await expectPersistedFailure();
	});

	test("keeps the failure available if the chat closes while loading its draft", async () => {
		let resolveDraft!: (message: IMessage | undefined) => void;
		const draft = new Promise<IMessage | undefined>((resolve) => {
			resolveDraft = resolve;
		});
		vi.mocked(latestChatDraft).mockImplementationOnce(() => draft);
		await failDetachedRun(failureMessage);
		await act(async () =>
			root.render(<ChatInterface appId="app" event={event} />),
		);
		await waitForChat(() => {
			expect(latestChatDraft).toHaveBeenCalledWith("session");
		});
		await act(async () => root.unmount());
		expect(state.engine.hasStream("session")).toBe(true);
		await act(async () => resolveDraft(undefined));
		root = createRoot(container);
		await expectPersistedFailure();
	});

	test("saves one error reply when an active run fails after reconnecting", async () => {
		await putUserMessage();
		let fail!: (failure: unknown) => void;
		const eventState: Pick<IEventState, "executeEvent"> = {
			executeEvent: () =>
				new Promise((_resolve, reject) => {
					fail = reject;
				}),
		};
		state.engine.setBackend({ eventState } as IBackendState);
		const subscribe = vi.spyOn(state.engine, "subscribeToEventStream");
		const running = state.engine.executeEvent("session", {
			appId: "app",
			eventId: "chat",
			payload: { id: "node" },
		});
		const rejected = expect(running).rejects.toBe(failureMessage);
		await act(async () =>
			root.render(<ChatInterface appId="app" event={event} />),
		);
		await waitForChat(() => {
			expect(subscribe).toHaveBeenCalledTimes(1);
		});
		await act(async () => {
			fail(failureMessage);
			await rejected;
		});
		await expectPersistedFailure();
		expect(subscribe).toHaveBeenCalledTimes(1);
	});
});
