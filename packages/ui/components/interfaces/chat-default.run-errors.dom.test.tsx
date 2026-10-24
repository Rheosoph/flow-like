// @vitest-environment happy-dom

import "fake-indexeddb/auto";
import { type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { ExecutionEngineProvider } from "../../lib/execution-engine";
import type { ExecuteEventFn } from "../../lib/execution-engine";
import type { IIntercomEvent } from "../../lib/schema/events/intercom-event";
import type { ILogMetadata } from "../../lib/schema/flow/log-metadata";
import type { IBackendState } from "../../state/backend-state";
import { ChatInterface } from "./chat-default";
import {
	type IMessage,
	chatDb,
	decodeMessageRow,
} from "./chat-default/chat-db";
import type { ISendMessageFunction } from "./chat-default/chatbox";
import type { IUseInterfaceProps } from "./interfaces";

const state = vi.hoisted(() => ({
	engine: undefined as unknown as ExecutionEngineProvider,
	backend: {} as IBackendState,
	router: { push: vi.fn(), replace: vi.fn() },
	params: new URLSearchParams({ sessionId: "session" }),
	setQueryParams: vi.fn(),
	send: undefined as ISendMessageFunction | undefined,
}));

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("next/navigation", () => ({
	usePathname: () => "/use",
	useRouter: () => state.router,
}));
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
	buildWorkflowFrontendContext: vi.fn(async () => ({})),
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
// Keep the composer lightweight while exercising the real error card and saved messages.
vi.mock("./chat-default/chat", async () => {
	const { ChatMessageError } = await import(
		"./chat-default/chat-message-error"
	);
	return {
		Chat: ({
			messages,
			onSendMessage,
		}: {
			messages: IMessage[];
			onSendMessage: ISendMessageFunction;
		}) => {
			state.send = onSendMessage;
			return (
				<div>
					{messages.map((message) => (
						<div key={message.id}>
							{typeof message.inner.content === "string"
								? message.inner.content
								: null}
							{message.error && <ChatMessageError error={message.error} />}
						</div>
					))}
				</div>
			);
		},
	};
});
vi.mock("./chat-default/attachment", () => ({
	fileToAttachment: vi.fn(async () => []),
}));
vi.mock("./chat-default/history", () => ({ ChatHistory: () => null }));
vi.mock("./chat-default/welcome", () => ({
	ChatWelcome: ({ onSendMessage }: { onSendMessage: ISendMessageFunction }) => {
		state.send = onSendMessage;
		return null;
	},
}));
vi.mock("sonner", () => ({ toast: { error: vi.fn() } }));
vi.mock("./chat-default/respond-interaction", () => ({
	submitInteractionResponse: vi.fn(),
}));

const event = {
	id: "chat",
	name: "Support",
	board_id: "board",
	node_id: "node",
	config: [],
} as unknown as IUseInterfaceProps["event"];
let root: Root;
let container: HTMLDivElement;
let emit: (events: IIntercomEvent[]) => void;
let finish: (metadata?: ILogMetadata) => void;
let reject: (failure: unknown) => void;

beforeEach(async () => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	state.send = undefined;
	state.engine = new ExecutionEngineProvider();
	const executeEvent: ExecuteEventFn = (
		_app,
		_event,
		_payload,
		_stream,
		_start,
		callback,
	) => {
		if (!callback) throw new Error("Missing event callback");
		emit = callback;
		return new Promise((resolve, fail) => {
			finish = resolve;
			reject = fail;
		});
	};
	state.backend = {
		isOffline: async () => true,
		eventState: { executeEvent },
	} as unknown as IBackendState;
	state.engine.setBackend(state.backend);
	for (const table of chatDb.tables) await table.clear();
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
	const report = console.error;
	vi.spyOn(console, "error").mockImplementation((...args) => {
		if (args[0] !== "Execution error:" && args[0] !== "Error sending message:")
			report(...args);
	});
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.restoreAllMocks();
});

async function waitForChat(assertion: () => void | Promise<void>) {
	await vi.waitFor(async () => {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 10));
		});
		await assertion();
	});
}

function terminal(payload: unknown, event_type = "completed"): IIntercomEvent {
	return { event_id: event_type, event_type, payload } as IIntercomEvent;
}

const failedMetadata = (error_message?: string) =>
	({ run_id: "run", log_level: 4, error_message }) as ILogMetadata;

async function mount() {
	await act(async () =>
		root.render(<ChatInterface appId="app" event={event} />),
	);
	await waitForChat(() => expect(state.send).toBeDefined());
}

async function send() {
	await mount();
	let sending!: Promise<void>;
	await act(async () => {
		if (!state.send) throw new Error("Chat is not mounted");
		sending = state.send("Hello");
	});
	await waitForChat(() =>
		expect(state.engine.isStreamActive("session")).toBe(true),
	);
	return { sending };
}

async function expectSavedError(message: string) {
	await waitForChat(async () => {
		const replies = (await chatDb.messages.toArray())
			.map(decodeMessageRow)
			.filter((message) => message.inner.role === "assistant");
		expect(replies).toHaveLength(1);
		expect(replies[0].error?.message).toBe(message);
		expect(container.querySelectorAll('[role="alert"]')).toHaveLength(1);
		expect(container.querySelector('[role="alert"]')?.textContent).toContain(
			message,
		);
		expect(container.textContent).toContain(
			"Support could not finish this response",
		);
	});
}

describe("chat run error reporting", () => {
	test.each([
		[
			"native completion",
			[terminal({ status: "failed", log_level: 4 })],
			failedMetadata(),
		],
		["hosted completion", [terminal({ status: "failed" })], undefined],
	] as const)(
		"shows an inline error for %s without response content",
		async (_name, events, meta) => {
			const { sending } = await send();
			await act(async () => {
				emit([...events]);
				finish(meta);
				await sending;
			});
			await expectSavedError(
				"The workflow failed before it could finish this response. Check the run logs for details.",
			);
		},
	);

	test("preserves partial content and the error after reopening the chat", async () => {
		const { sending } = await send();
		await act(async () => {
			emit([
				{
					event_id: "partial",
					event_type: "chat_stream_partial",
					payload: {
						chunk: {
							choices: [
								{
									index: 0,
									delta: { role: "assistant", content: "Partial answer" },
								},
							],
						},
					},
				} as IIntercomEvent,
				terminal({ message: "DependencyFailed" }, "error"),
			]);
			finish();
			await sending;
		});
		await expectSavedError("DependencyFailed");
		expect(container.textContent).toContain("Partial answer");
		await act(async () => root.unmount());
		root = createRoot(container);
		await mount();
		await expectSavedError("DependencyFailed");
		expect(container.textContent).toContain("Partial answer");
	});

	test("shows a resolved failed summary when the chat reconnects after completion", async () => {
		const running = state.engine.executeEvent("session", {
			appId: "app",
			eventId: "chat",
			payload: { id: "node" },
		});
		finish(failedMetadata("DependencyFailed"));
		await running;
		await mount();
		await expectSavedError("DependencyFailed");
	});

	test("shows a preamble rejection before the first streamed event", async () => {
		const { sending } = await send();
		await act(async () => {
			reject({
				error: "The active profile changed before automation could start.",
			});
			await sending;
		});
		await expectSavedError(
			"The active profile changed before automation could start.",
		);
	});

	test("keeps a successfully handled node error out of the chat", async () => {
		const { sending } = await send();
		await act(async () => {
			emit([terminal({ status: "completed", log_level: 3 })]);
			finish({ log_level: 3 } as ILogMetadata);
			await sending;
		});
		await waitForChat(async () => {
			const replies = (await chatDb.messages.toArray())
				.map(decodeMessageRow)
				.filter((message) => message.inner.role === "assistant");
			expect(replies).toHaveLength(1);
			expect(replies[0].error).toBeUndefined();
		});
		expect(container.querySelector('[role="alert"]')).toBeNull();
	});
});
