import {
	BOARD_FORMAT_HEADER,
	CURRENT_BOARD_FORMAT_VERSION,
} from "@flow-like/flow-like-ui/lib/board-format";
import legacyBoardFixture from "@flow-like/flow-like-ui/lib/flowpilot/fixtures/copilot-board-reference.json";
import type { IChannelHandle } from "@flow-like/flow-like-ui/lib/schema/channel";
import type { CopilotToolContext } from "@flow-like/flow-like-ui/lib/schema/copilot";
import type { IBoard } from "@flow-like/flow-like-ui/lib/schema/flow/board";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	cancelChannel: vi.fn<(handle: unknown) => Promise<void>>(async () => {}),
	dispatchSpecialistToolRequest: vi.fn(async () => undefined),
	isChannelHandle: vi.fn((value: unknown) => Boolean(value)),
}));

vi.mock("@flow-like/flow-like-ui", () => ({}));
vi.mock("@flow-like/flow-like-ui/lib/channel", () => ({
	cancelChannel: mocks.cancelChannel,
	isChannelHandle: mocks.isChannelHandle,
}));
vi.mock(
	"@flow-like/flow-like-ui/state/global-chat/global-chat-run-control",
	() => ({ globalChatTransportRunId: vi.fn() }),
);
vi.mock(
	"@flow-like/flow-like-ui/state/global-chat/global-chat-tool-registry",
	() => ({ runGlobalChatTool: vi.fn() }),
);
vi.mock(
	"@flow-like/flow-like-ui/state/global-chat/global-chat-web-transport",
	() => ({
		dispatchSpecialistToolRequest: mocks.dispatchSpecialistToolRequest,
	}),
);
vi.mock("sonner", () => ({ toast: vi.fn() }));
vi.mock("../oauth-db", () => ({
	oauthConsentStore: {},
	oauthTokenStore: {},
}));
vi.mock("../oauth-service", () => ({
	getOAuthApiBaseUrl: vi.fn(),
	getOAuthService: vi.fn(),
}));
vi.mock("./api-utils", () => ({
	apiDelete: vi.fn(),
	apiGet: vi.fn(),
	apiPatch: vi.fn(),
	apiPost: vi.fn(),
	apiPut: vi.fn(),
	getApiBaseUrl: () => "https://api.example.test",
}));

import { WebBoardState } from "./board-state";

const encoder = new TextEncoder();

function channel(id: string): IChannelHandle {
	return {
		channel_id: id,
		expires_at: 4_000_000_000,
		transport: {
			type: "http",
			push_url: `https://api.example.test/channels/${id}`,
			token: "channel-token",
		},
	};
}

function copilotStream(runChannel: IChannelHandle, announceRun = true) {
	let controller: ReadableStreamDefaultController<Uint8Array> | undefined;
	const cancel = vi.fn();
	const sendRun = () =>
		controller?.enqueue(
			encoder.encode(
				`event: run\ndata: ${JSON.stringify({ runId: runChannel.channel_id, channel: runChannel })}\n\n`,
			),
		);
	const body = new ReadableStream<Uint8Array>({
		start(streamController) {
			controller = streamController;
			if (announceRun) sendRun();
		},
		cancel,
	});
	return {
		cancel,
		response: new Response(body, {
			headers: { "content-type": "text/event-stream" },
		}),
		failOnAbort(signal: AbortSignal) {
			signal.addEventListener(
				"abort",
				() => controller?.error(new DOMException("Aborted", "AbortError")),
				{ once: true },
			);
		},
		sendRun,
		close() {
			controller?.close();
		},
		sendRaw(frame: string) {
			controller?.enqueue(encoder.encode(frame));
		},
		sendToken(token: string) {
			controller?.enqueue(encoder.encode(`event: token\ndata: ${token}\n\n`));
		},
		sendToolRequest(request: Record<string, unknown>) {
			controller?.enqueue(
				encoder.encode(
					`event: tool_request\ndata: ${JSON.stringify(request)}\n\n`,
				),
			);
		},
		finish(result: unknown = { message: "done" }, close = true) {
			controller?.enqueue(
				encoder.encode(`event: final\ndata: ${JSON.stringify(result)}\n\n`),
			);
			if (close) controller?.close();
		},
	};
}

function startChat(
	state: WebBoardState,
	requestId: string,
	onToken: (token: string) => void = () => undefined,
	toolContext?: CopilotToolContext,
) {
	return state.copilot_chat(
		"DataStudio",
		null,
		undefined,
		[],
		null,
		null,
		[],
		"Find active customers",
		[],
		undefined,
		onToken,
		"openai:gpt-5",
		undefined,
		undefined,
		undefined,
		undefined,
		true,
		true,
		toolContext,
		requestId,
	);
}

describe("WebBoardState Copilot requests", () => {
	afterEach(() => {
		vi.unstubAllGlobals();
	});

	test.each([false, true])(
		"sends a compact board reference with format and model selection (stream=%s)",
		async (streaming) => {
			const board = {
				id: "board-geometry",
				format_version: 2,
				nodes: {},
				description: "x".repeat(6 * 1024 * 1024),
			} as IBoard;
			expect(encoder.encode(JSON.stringify(board)).byteLength).toBeGreaterThan(
				6 * 1024 * 1024,
			);
			const result = {
				message: "Board loaded",
				active_scope: "Board",
				components: [],
			};
			const stream = copilotStream(channel("run-board-format"));
			stream.finish(result);
			const fetchMock = vi.fn(async (_url: string, init?: RequestInit) => {
				const headers = new Headers(init?.headers);
				// The API defaults missing format headers to 1 before reloading the board.
				const supported = Number(headers.get(BOARD_FORMAT_HEADER) ?? 1);
				if (supported < (board.format_version ?? 1)) {
					return Response.json(
						{ error: { code: "BOARD_FORMAT_UPGRADE_REQUIRED" } },
						{ status: 426 },
					);
				}
				return streaming ? stream.response : Response.json(result);
			});
			vi.stubGlobal("fetch", fetchMock);
			const state = new WebBoardState({
				auth: { user: { access_token: "client-token" } },
				profile: { id: "selected-profile" },
			} as never);

			await expect(
				state.copilot_chat(
					"Board",
					board,
					undefined,
					[],
					null,
					null,
					[],
					"Explain this board",
					[],
					undefined,
					streaming ? () => undefined : undefined,
					"selected-model-bit",
					undefined,
					undefined,
					undefined,
					undefined,
					undefined,
					undefined,
					{ appId: "app-1", boardId: board.id },
				),
			).resolves.toEqual(result);

			const [url, init] = fetchMock.mock.calls[0];
			expect(url).toBe("https://api.example.test/api/v1/ai/copilot/chat");
			const headers = new Headers(init?.headers);
			expect(headers.get(BOARD_FORMAT_HEADER)).toBe(
				String(CURRENT_BOARD_FORMAT_VERSION),
			);
			expect(headers.get("Authorization")).toBe("Bearer client-token");
			expect(headers.get("Accept")).toBe(
				streaming ? "text/event-stream" : null,
			);
			const body = init?.body as string;
			expect(encoder.encode(body).byteLength).toBeLessThan(2_048);
			expect(JSON.parse(body).board).toEqual({
				...legacyBoardFixture,
				id: board.id,
			});
			expect(JSON.parse(body)).toMatchObject({
				board_id: board.id,
				app_id: "app-1",
				profile_id: "selected-profile",
				model_id: "selected-model-bit",
				stream: streaming,
			});
		},
	);

	test.each([false, true])(
		"rejects a pages-only specialist response (stream=%s)",
		async (streaming) => {
			const result = {
				message: "I can only create pages",
				active_scope: "Frontend",
				components: [],
			};
			const stream = copilotStream(channel("misrouted-board"));
			stream.finish(result);
			vi.stubGlobal(
				"fetch",
				vi.fn(async () =>
					streaming ? stream.response : Response.json(result),
				),
			);
			const state = new WebBoardState({ auth: undefined } as never);
			await expect(
				state.copilot_chat(
					"Board",
					{ id: "board-a" } as IBoard,
					undefined,
					[],
					null,
					null,
					[],
					"Build a workflow",
					[],
					undefined,
					streaming ? () => undefined : undefined,
				),
			).rejects.toMatchObject({ code: "FLOWPILOT_BOARD_SCOPE_MISMATCH" });
		},
	);

	test("uses the explicit Board context when no snapshot is supplied", async () => {
		const fetchMock = vi.fn(async (_url: string, _init?: RequestInit) =>
			Response.json({
				message: "Board inspected",
				active_scope: "Board",
				components: [],
			}),
		);
		vi.stubGlobal("fetch", fetchMock);
		const state = new WebBoardState({ auth: undefined } as never);
		await state.copilot_chat(
			"Board",
			null,
			undefined,
			[],
			null,
			null,
			[],
			"Inspect the workflow",
			[],
			undefined,
			undefined,
			undefined,
			undefined,
			undefined,
			undefined,
			undefined,
			true,
			true,
			{ appId: "app-1", boardId: "board-context" },
		);
		const payload = JSON.parse(fetchMock.mock.calls[0][1]?.body as string);
		expect(payload.board_id).toBe("board-context");
		expect(payload.board).toEqual({
			...legacyBoardFixture,
			id: "board-context",
		});
	});

	test("rejects missing or conflicting Board identity before dispatch", async () => {
		const fetchMock = vi.fn();
		vi.stubGlobal("fetch", fetchMock);
		const state = new WebBoardState({ auth: undefined } as never);
		for (const board of [null, { id: "different-board" } as IBoard]) {
			await expect(
				state.copilot_chat(
					"Board",
					board,
					undefined,
					[],
					null,
					null,
					[],
					"Build a workflow",
					[],
					undefined,
					undefined,
					undefined,
					undefined,
					undefined,
					undefined,
					undefined,
					true,
					false,
					board ? { boardId: "requested-board" } : undefined,
				),
			).rejects.toThrow(board ? "does not match" : "requires a board id");
		}
		expect(fetchMock).not.toHaveBeenCalled();
	});

	test("keeps requests without a board free of board context", async () => {
		const fetchMock = vi.fn(async (_url: string, _init?: RequestInit) =>
			Response.json({ message: "Query prepared" }),
		);
		vi.stubGlobal("fetch", fetchMock);
		const state = new WebBoardState({ auth: undefined } as never);

		await startChat(state, "query-without-board", undefined, {
			appId: "app-1",
			boardId: "unrelated-board",
		});

		const body = JSON.parse(fetchMock.mock.calls[0][1]?.body as string);
		expect(body.app_id).toBe("app-1");
		expect(body).not.toHaveProperty("board");
		expect(body).not.toHaveProperty("board_id");
	});
});

describe("WebBoardState Copilot terminal frames", () => {
	beforeEach(() => vi.clearAllMocks());
	afterEach(() => vi.unstubAllGlobals());

	test("returns at the final frame without waiting for EOF or running later tools", async () => {
		const stream = copilotStream(channel("final-open-stream"));
		vi.stubGlobal(
			"fetch",
			vi.fn(async () => stream.response),
		);
		const state = new WebBoardState({ auth: undefined } as never);
		const chat = startChat(state, "final-open");
		stream.sendRaw(
			'event: final\ndata: {"message":"done"}\n\nevent: tool_request\ndata: {"requestId":"late-tool"}\n\n',
		);
		await expect(chat).resolves.toMatchObject({ message: "done" });
		expect(stream.cancel).toHaveBeenCalledOnce();
		expect(mocks.dispatchSpecialistToolRequest).not.toHaveBeenCalled();
		expect(mocks.cancelChannel).not.toHaveBeenCalled();
	});

	test("a failed tool delivery wakes an idle reader and cancels the server run", async () => {
		const runChannel = channel("failed-delivery");
		const stream = copilotStream(runChannel);
		let rejectDelivery!: (error: Error) => void;
		mocks.dispatchSpecialistToolRequest.mockImplementationOnce(
			() =>
				new Promise((_resolve, reject) => {
					rejectDelivery = reject;
				}),
		);
		vi.stubGlobal(
			"fetch",
			vi.fn(async () => stream.response),
		);
		const state = new WebBoardState({ auth: undefined } as never);
		const chat = startChat(state, "delivery-failure");
		const failed = expect(chat).rejects.toThrow("result delivery failed");
		stream.sendToolRequest({ requestId: "tool-1", toolName: "database_tool" });
		await vi.waitFor(() =>
			expect(mocks.dispatchSpecialistToolRequest).toHaveBeenCalledOnce(),
		);
		rejectDelivery(new Error("result delivery failed"));
		await failed;
		expect(stream.cancel).toHaveBeenCalledOnce();
		expect(mocks.cancelChannel).toHaveBeenCalledWith(runChannel);
	});

	test("cancellation interrupts a pending tool delivery after the final frame", async () => {
		const stream = copilotStream(channel("cancel-pending-delivery"));
		let resolveDelivery!: () => void;
		mocks.dispatchSpecialistToolRequest.mockImplementationOnce(
			() =>
				new Promise((resolve) => {
					resolveDelivery = () => resolve(undefined);
				}),
		);
		vi.stubGlobal(
			"fetch",
			vi.fn(async () => stream.response),
		);
		const state = new WebBoardState({ auth: undefined } as never);
		const chat = startChat(state, "cancel-delivery");
		const cancelled = expect(chat).rejects.toMatchObject({
			name: "AbortError",
		});
		stream.sendToolRequest({ requestId: "tool-1", toolName: "database_tool" });
		await vi.waitFor(() =>
			expect(mocks.dispatchSpecialistToolRequest).toHaveBeenCalledOnce(),
		);
		stream.finish({ message: "done" }, false);
		await state.cancelCopilotChat("cancel-delivery");
		await cancelled;
		expect(stream.cancel).toHaveBeenCalledOnce();
		resolveDelivery();
	});

	test("EOF without a final frame fails without waiting for an outstanding tool", async () => {
		const runChannel = channel("missing-final");
		const stream = copilotStream(runChannel);
		let resolveDelivery!: () => void;
		mocks.dispatchSpecialistToolRequest.mockImplementationOnce(
			() =>
				new Promise((resolve) => {
					resolveDelivery = () => resolve(undefined);
				}),
		);
		vi.stubGlobal(
			"fetch",
			vi.fn(async () => stream.response),
		);
		const state = new WebBoardState({ auth: undefined } as never);
		const chat = startChat(state, "missing-final");
		const failed = expect(chat).rejects.toThrow("without a final event");
		stream.sendToolRequest({ requestId: "tool-1", toolName: "database_tool" });
		await vi.waitFor(() =>
			expect(mocks.dispatchSpecialistToolRequest).toHaveBeenCalledOnce(),
		);
		stream.close();
		await failed;
		expect(mocks.cancelChannel).toHaveBeenCalledWith(runChannel);
		resolveDelivery();
	});

	test.each([false, true])(
		"malformed Board final payload reports a routing error (stream=%s)",
		async (streaming) => {
			const stream = copilotStream(channel("malformed-board"));
			stream.finish(null, false);
			vi.stubGlobal(
				"fetch",
				vi.fn(async () => (streaming ? stream.response : Response.json(null))),
			);
			const state = new WebBoardState({ auth: undefined } as never);
			await expect(
				state.copilot_chat(
					"Board",
					{ id: "board-a" } as IBoard,
					undefined,
					[],
					null,
					null,
					[],
					"Inspect",
					[],
					undefined,
					streaming ? () => undefined : undefined,
				),
			).rejects.toMatchObject({ code: "FLOWPILOT_BOARD_SCOPE_MISMATCH" });
			if (streaming) expect(stream.cancel).toHaveBeenCalledOnce();
		},
	);

	test("a terminal error closes a stream that remains open", async () => {
		const stream = copilotStream(channel("error-open-stream"));
		vi.stubGlobal(
			"fetch",
			vi.fn(async () => stream.response),
		);
		const state = new WebBoardState({ auth: undefined } as never);
		const chat = startChat(state, "terminal-error");
		const failed = expect(chat).rejects.toThrow("provider failed");
		stream.sendRaw('event: error\ndata: {"error":"provider failed"}\n\n');
		await failed;
		expect(stream.cancel).toHaveBeenCalledOnce();
	});
});

describe("WebBoardState Copilot cancellation", () => {
	beforeEach(() => {
		vi.clearAllMocks();
	});

	test("cancels the server run through its channel before aborting the stream", async () => {
		const runChannel = channel("run-1");
		const stream = copilotStream(runChannel);
		let requestSignal: AbortSignal | undefined;
		vi.stubGlobal(
			"fetch",
			vi.fn(async (_url: string, init?: RequestInit) => {
				requestSignal = init?.signal as AbortSignal;
				stream.failOnAbort(requestSignal);
				return stream.response;
			}),
		);
		let signalWasAbortedAtDelivery: boolean | undefined;
		mocks.cancelChannel.mockImplementationOnce(async () => {
			signalWasAbortedAtDelivery = requestSignal?.aborted;
		});

		const state = new WebBoardState({ auth: undefined } as never);
		const chat = startChat(state, "query-1").catch((error) => error);
		await vi.waitFor(() => expect(mocks.isChannelHandle).toHaveBeenCalled());

		await state.cancelCopilotChat("query-1");

		expect(mocks.cancelChannel).toHaveBeenCalledOnce();
		expect(mocks.cancelChannel).toHaveBeenCalledWith(runChannel);
		expect(signalWasAbortedAtDelivery).toBe(false);
		expect(requestSignal?.aborted).toBe(true);
		expect(await chat).toBeInstanceOf(Error);
	});

	test("keeps a just-started stream alive long enough to receive its cancel channel", async () => {
		const runChannel = channel("run-delayed");
		const stream = copilotStream(runChannel, false);
		let requestSignal: AbortSignal | undefined;
		vi.stubGlobal(
			"fetch",
			vi.fn(async (_url: string, init?: RequestInit) => {
				requestSignal = init?.signal as AbortSignal;
				stream.failOnAbort(requestSignal);
				return stream.response;
			}),
		);
		const onToken = vi.fn();
		const state = new WebBoardState({ auth: undefined } as never);
		const chat = startChat(state, "query-delayed", onToken).catch(
			(error) => error,
		);
		await vi.waitFor(() => expect(fetch).toHaveBeenCalled());

		const cancellation = state.cancelCopilotChat("query-delayed");
		expect(requestSignal?.aborted).toBe(false);
		stream.sendToken("must not reach the caller");
		stream.sendRun();
		await cancellation;

		expect(onToken).not.toHaveBeenCalled();
		expect(mocks.cancelChannel).toHaveBeenCalledWith(runChannel);
		expect(requestSignal?.aborted).toBe(true);
		expect(await chat).toBeInstanceOf(Error);
	});

	test("a stale stream cannot clear the control for its replacement", async () => {
		const firstChannel = channel("run-1");
		const secondChannel = channel("run-2");
		const streams = [copilotStream(firstChannel), copilotStream(secondChannel)];
		let fetchIndex = 0;
		vi.stubGlobal(
			"fetch",
			vi.fn(async (_url: string, init?: RequestInit) => {
				const stream = streams[fetchIndex++];
				stream.failOnAbort(init?.signal as AbortSignal);
				return stream.response;
			}),
		);

		const state = new WebBoardState({ auth: undefined } as never);
		const first = startChat(state, "query-1").catch((error) => error);
		await vi.waitFor(() =>
			expect(mocks.isChannelHandle).toHaveBeenCalledTimes(1),
		);
		const second = startChat(state, "query-1").catch((error) => error);
		await vi.waitFor(() =>
			expect(mocks.isChannelHandle).toHaveBeenCalledTimes(2),
		);

		await state.cancelCopilotChat("query-1");

		expect(mocks.cancelChannel.mock.calls.map(([handle]) => handle)).toEqual([
			firstChannel,
			secondChannel,
		]);
		expect(await first).toBeInstanceOf(Error);
		expect(await second).toBeInstanceOf(Error);
	});

	test("carries the pinned Home profile into model resolution and nested tool dispatch", async () => {
		const runChannel = channel("run-home");
		const stream = copilotStream(runChannel);
		vi.stubGlobal(
			"fetch",
			vi.fn(async () => stream.response),
		);

		const state = new WebBoardState({
			auth: undefined,
			profile: { id: "profile-b" },
		} as never);
		const chat = startChat(state, "home-agent", () => undefined, {
			parentRequestId: "outer-home-1",
			profileId: "profile-a",
		});
		await vi.waitFor(() => expect(mocks.isChannelHandle).toHaveBeenCalled());
		expect(
			JSON.parse(vi.mocked(fetch).mock.calls[0][1]?.body as string),
		).toMatchObject({ profile_id: "profile-a" });
		stream.sendToolRequest({
			requestId: "home-apply-1",
			toolName: "apply_home_layout",
			arguments: {},
			channel: channel("home-tool"),
		});

		await vi.waitFor(() =>
			expect(mocks.dispatchSpecialistToolRequest).toHaveBeenCalledWith(
				expect.objectContaining({
					parentRequestId: "outer-home-1",
					profileId: "profile-a",
				}),
			),
		);
		stream.finish();
		await chat;
	});

	test("sends the selected profile when the caller has no pinned Home context", async () => {
		const stream = copilotStream(channel("run-selected-profile"));
		vi.stubGlobal(
			"fetch",
			vi.fn(async () => stream.response),
		);
		const state = new WebBoardState({
			auth: undefined,
			profile: { id: "selected-profile" },
		} as never);
		const chat = startChat(state, "selected-profile-agent");
		await vi.waitFor(() => expect(fetch).toHaveBeenCalled());
		expect(
			JSON.parse(vi.mocked(fetch).mock.calls[0][1]?.body as string),
		).toMatchObject({ profile_id: "selected-profile" });
		stream.finish();
		await chat;
	});
});
