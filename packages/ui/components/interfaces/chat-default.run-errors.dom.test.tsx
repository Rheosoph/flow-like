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
	putChatMessage,
} from "./chat-default/chat-db";
import {
	type IChatMessageInteraction,
	createMessageReference,
} from "./chat-default/chat-message-interaction";
import type { ISendMessageFunction } from "./chat-default/chatbox";
import { joinContentText } from "./chat-default/inline-segments";
import type { IUseInterfaceProps } from "./interfaces";

const state = vi.hoisted(() => ({
	engine: undefined as unknown as ExecutionEngineProvider,
	backend: {} as IBackendState,
	router: { push: vi.fn(), replace: vi.fn() },
	params: new URLSearchParams({ sessionId: "session" }),
	setQueryParams: vi.fn(),
	send: undefined as ISendMessageFunction | undefined,
	selectedModel: undefined as string | undefined,
	react: undefined as
		| ((
				message: IMessage,
				emoji: string,
				operation: "add" | "remove",
		  ) => Promise<void>)
		| undefined,
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
			onMessageReaction,
		}: {
			messages: IMessage[];
			onSendMessage: ISendMessageFunction;
			onMessageReaction: typeof state.react;
		}) => {
			state.send = onSendMessage;
			state.react = onMessageReaction;
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
vi.mock("./chat-default/use-chat-model", () => ({
	useChatModel: () => ({ selectedModel: state.selectedModel }),
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
let requests: Parameters<ExecuteEventFn>[2][];
let oauthRequests: Record<string, unknown>[];
const recordOAuth = (event: Event) => {
	oauthRequests.push((event as CustomEvent<Record<string, unknown>>).detail);
};

beforeEach(async () => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	state.send = undefined;
	state.selectedModel = undefined;
	state.react = undefined;
	state.params = new URLSearchParams({ sessionId: "session" });
	requests = [];
	oauthRequests = [];
	window.addEventListener("flow:oauth-required", recordOAuth);
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
		requests.push(_payload);
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
	window.removeEventListener("flow:oauth-required", recordOAuth);
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

describe("chat model requests", () => {
	test.each([undefined, "chosen-model"])(
		"sends the selected model only when present: %s",
		async (selectedModel) => {
			state.selectedModel = selectedModel;
			const { sending } = await send();
			await waitForChat(() => expect(requests).toHaveLength(1));
			if (selectedModel) {
				expect(requests[0].payload).toHaveProperty(
					"selected_model",
					selectedModel,
				);
			} else {
				expect(requests[0].payload).not.toHaveProperty("selected_model");
			}
			await act(async () => {
				finish();
				await sending;
			});
		},
	);
});

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

describe("quoted replies and reactions", () => {
	function required<T>(value: T | undefined): T {
		if (value === undefined) throw new Error("Expected a value");
		return value;
	}
	const target: IMessage = {
		id: "older-model-message",
		appId: "app",
		sessionId: "session",
		timestamp: 1,
		files: [],
		inner: {
			role: "assistant",
			content: "The first proposal uses a blue background.",
		} as IMessage["inner"],
	};

	const payload = (index: number) =>
		requests[index].payload as {
			messages: { role: string; content: IMessage["inner"]["content"] }[];
		};

	async function finishRun(sending?: Promise<void>) {
		await act(async () => {
			finish();
			await sending;
		});
		await waitForChat(() =>
			expect(state.engine.isStreamActive("session")).toBe(false),
		);
	}

	test("a reply outside the history window starts a run and keeps its context in later history", async () => {
		await putChatMessage(target);
		for (let index = 0; index < 6; index++) {
			await putChatMessage({
				...target,
				id: `later-${index}`,
				timestamp: index + 2,
				inner: { ...target.inner, content: `Later answer ${index}` },
			});
		}
		await mount();
		const interaction: IChatMessageInteraction = {
			type: "reply",
			target: createMessageReference(target),
		};
		let sending!: Promise<void>;
		await act(async () => {
			sending = required(state.send)(
				"Please use green instead.",
				undefined,
				undefined,
				undefined,
				interaction,
			);
		});
		await waitForChat(() => expect(requests).toHaveLength(1));
		const outgoing = payload(0).messages;
		expect(outgoing).toHaveLength(6);
		expect(outgoing.at(-1)?.role).toBe("user");
		const text = joinContentText(required(outgoing.at(-1)).content);
		expect(text).toContain(target.id);
		expect(text).toContain("The first proposal uses a blue background.");
		expect(text).toContain("Please use green instead.");
		expect(
			outgoing
				.slice(0, -1)
				.every(
					(message) =>
						!joinContentText(message.content).includes("first proposal"),
				),
		).toBe(true);
		const user = required(
			(await chatDb.messages.toArray())
				.map(decodeMessageRow)
				.find((message) => message.inner.role === "user"),
		);
		expect(joinContentText(user.inner.content)).toBe(
			"Please use green instead.",
		);
		expect(user.interaction).toEqual(interaction);
		await finishRun(sending);

		await act(async () => {
			sending = required(state.send)("Continue.");
		});
		await waitForChat(() => expect(requests).toHaveLength(2));
		expect(
			payload(1).messages.some(
				(message) =>
					message.role === "user" &&
					joinContentText(message.content).includes(target.id),
			),
		).toBe(true);
		await finishRun(sending);
	});

	test("adding and removing a reaction each starts one run and saves the literal action", async () => {
		await putChatMessage(target);
		await mount();
		for (const [index, operation] of ["add", "remove"].entries()) {
			let sending!: Promise<void>;
			await act(async () => {
				sending = required(state.react)(
					target,
					"👍",
					operation as "add" | "remove",
				);
			});
			await waitForChat(() => expect(requests).toHaveLength(index + 1));
			const outgoing = required(payload(index).messages.at(-1));
			expect(outgoing.role).toBe("user");
			expect(joinContentText(outgoing.content)).toContain("👍");
			expect(joinContentText(outgoing.content)).toContain(target.id);
			const stored = decodeMessageRow(
				required(await chatDb.messages.get(target.id)),
			);
			expect(stored.reactions).toEqual(operation === "add" ? ["👍"] : []);
			const users = (await chatDb.messages.toArray())
				.map(decodeMessageRow)
				.filter((message) => message.inner.role === "user");
			expect(users).toHaveLength(index + 1);
			const action = required(
				users.find(
					(message) =>
						message.interaction?.type === "reaction" &&
						message.interaction.operation === operation,
				),
			);
			expect(action.interaction).toMatchObject({
				type: "reaction",
				operation,
				emoji: "👍",
				target: { messageId: target.id },
			});
			expect(joinContentText(action.inner.content)).toBe("👍");
			await finishRun(sending);
		}
	});

	test("a busy chat rejects reactions without persisting or silently dropping them", async () => {
		await putChatMessage(target);
		const { sending } = await send();
		await act(async () => {
			await expect(required(state.react)(target, "❤️", "add")).rejects.toThrow(
				"Please wait",
			);
		});
		expect(requests).toHaveLength(1);
		expect(
			decodeMessageRow(required(await chatDb.messages.get(target.id)))
				.reactions,
		).toBeUndefined();
		await finishRun(sending);
	});

	test("preflight failures reject a quoted reply instead of reporting a successful send", async () => {
		await putChatMessage(target);
		await mount();
		state.backend.isOffline = vi
			.fn()
			.mockRejectedValue(new Error("Offline lookup failed"));
		await act(async () => {
			await expect(
				required(state.send)(
					"Please clarify",
					undefined,
					undefined,
					undefined,
					{
						type: "reply",
						target: createMessageReference(target),
					},
				),
			).rejects.toThrow("Offline lookup failed");
		});
		expect(requests).toHaveLength(0);
		expect(await chatDb.messages.count()).toBe(1);
	});

	test("a missing reaction target rejects without saving an action or starting a run", async () => {
		await putChatMessage(target);
		await mount();
		await chatDb.messages.delete(target.id);
		await act(async () => {
			await expect(required(state.react)(target, "👍", "add")).rejects.toThrow(
				"no longer available",
			);
		});
		expect(requests).toHaveLength(0);
		expect(await chatDb.messages.count()).toBe(0);
	});

	test("OAuth retry retains reaction context and saves it only when the run is accepted", async () => {
		state.selectedModel = "original-model";
		await putChatMessage(target);
		state.backend.eventState.checkEventOAuth = vi
			.fn()
			.mockResolvedValue({ missingProviders: [{ provider: "example" }] });
		await mount();
		await act(async () => {
			await required(state.react)(target, "🎉", "add");
		});
		expect(requests).toHaveLength(0);
		await act(async () => {
			await expect(required(state.react)(target, "❤️", "add")).rejects.toThrow(
				"authorization request",
			);
		});
		expect(
			decodeMessageRow(required(await chatDb.messages.get(target.id)))
				.reactions,
		).toBeUndefined();
		state.selectedModel = "next-model";
		await mount();
		await act(async () => {
			window.dispatchEvent(
				new CustomEvent("flow:oauth-retry", {
					detail: { ...oauthRequests[0], skipConsentCheck: true },
				}),
			);
		});
		await waitForChat(() => expect(requests).toHaveLength(1));
		expect(requests[0].payload).toHaveProperty(
			"selected_model",
			"original-model",
		);
		expect(
			joinContentText(required(payload(0).messages.at(-1)).content),
		).toContain(target.id);
		expect(
			decodeMessageRow(required(await chatDb.messages.get(target.id)))
				.reactions,
		).toEqual(["🎉"]);
		await finishRun();
	});

	test("OAuth retry drops a pending quote when the user switched conversations", async () => {
		await putChatMessage(target);
		state.backend.eventState.checkEventOAuth = vi
			.fn()
			.mockResolvedValue({ missingProviders: [{ provider: "example" }] });
		await mount();
		await act(async () => {
			await required(state.send)(
				"Please clarify",
				undefined,
				undefined,
				undefined,
				{ type: "reply", target: createMessageReference(target) },
			);
		});
		state.params = new URLSearchParams({ sessionId: "another-session" });
		await mount();
		await act(async () => {
			window.dispatchEvent(
				new CustomEvent("flow:oauth-retry", {
					detail: { ...oauthRequests[0], skipConsentCheck: true },
				}),
			);
		});
		expect(requests).toHaveLength(0);
		expect(await chatDb.messages.count()).toBe(1);
	});

	test("cancelling the matching OAuth request allows a new interaction", async () => {
		await putChatMessage(target);
		state.backend.eventState.checkEventOAuth = vi
			.fn()
			.mockResolvedValue({ missingProviders: [{ provider: "example" }] });
		await mount();
		await act(async () => {
			await required(state.react)(target, "🎉", "add");
		});
		await act(async () => {
			window.dispatchEvent(
				new CustomEvent("flow:oauth-cancelled", {
					detail: { ...oauthRequests[0], requestId: "another-request" },
				}),
			);
			await expect(required(state.react)(target, "❤️", "add")).rejects.toThrow(
				"authorization request",
			);
			window.dispatchEvent(
				new CustomEvent("flow:oauth-cancelled", { detail: oauthRequests[0] }),
			);
		});
		state.backend.eventState.checkEventOAuth = vi
			.fn()
			.mockResolvedValue({ missingProviders: [] });
		let sending!: Promise<void>;
		await act(async () => {
			sending = required(state.react)(target, "❤️", "add");
		});
		await waitForChat(() => expect(requests).toHaveLength(1));
		expect(
			decodeMessageRow(required(await chatDb.messages.get(target.id)))
				.reactions,
		).toEqual(["❤️"]);
		await finishRun(sending);
	});
});
