import type { IChannelHandle } from "@flow-like/flow-like-ui/lib/schema/channel";
import type { IAgentDebugEvent } from "@flow-like/flow-like-ui/state/global-chat/agent-debug-report";
import {
	getGlobalChatRunControl,
	globalChatTransportRunId,
} from "@flow-like/flow-like-ui/state/global-chat/global-chat-run-control";
import {
	dispatchSpecialistToolRequest,
	parseSseFrame,
	webGlobalChatStart,
} from "@flow-like/flow-like-ui/state/global-chat/global-chat-web-transport";
import { afterEach, describe, expect, test, vi } from "vitest";

const encoder = new TextEncoder();

function sseResponse(...chunks: string[]) {
	return new Response(
		new ReadableStream<Uint8Array>({
			start(controller) {
				for (const chunk of chunks) controller.enqueue(encoder.encode(chunk));
				controller.close();
			},
		}),
		{ status: 200, headers: { "content-type": "text/event-stream" } },
	);
}

function frame(event: string, data: unknown, trailingBlank = true) {
	const serialized = typeof data === "string" ? data : JSON.stringify(data);
	return `event: ${event}\ndata: ${serialized}${trailingBlank ? "\n\n" : ""}`;
}

function pushUrl(runId: string) {
	return `https://flow.example/api/v1/channels/${encodeURIComponent(runId)}/push`;
}

function httpHandle(runId: string, requestId?: string): IChannelHandle {
	return {
		channel_id: runId,
		request_id: requestId ?? null,
		expires_at: 4_102_444_800,
		transport: { type: "http", push_url: pushUrl(runId), token: "push-token" },
	};
}

function runFrame(runId: string) {
	return frame("run", { runId, channel: httpHandle(runId) });
}

function toolRequestFrame(
	runId: string,
	request: { requestId: string; toolName: string; arguments: unknown },
) {
	return frame("tool_request", {
		...request,
		channel: httpHandle(runId, request.requestId),
	});
}

function pushBody(init: RequestInit | undefined) {
	return JSON.parse(String(init?.body)) as {
		channel_id: string;
		request_id?: string;
		kind: string;
		value: Record<string, unknown>;
	};
}

afterEach(() => {
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
});

describe("browser global-chat transport", () => {
	test("parses multiline and CRLF SSE frames", () => {
		expect(
			parseSseFrame("event: token\r\ndata: first\r\ndata: second"),
		).toEqual({ event: "token", data: "first\nsecond" });
		expect(parseSseFrame(": keep-alive")).toBeNull();
	});

	test("flushes a final frame without a trailing blank line", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				sseResponse(
					runFrame("run-1"),
					frame("token", "hello"),
					frame("final", { message: "done" }, false),
				),
			);
		vi.stubGlobal("fetch", fetchMock);
		const chunks: string[] = [];

		const result = await webGlobalChatStart({
			baseUrl: "https://flow.example",
			body: { userPrompt: "hi" },
		})((chunk) => chunks.push(chunk));

		expect(result).toEqual({ message: "done" });
		expect(chunks).toEqual(["hello"]);
		expect(fetchMock).toHaveBeenCalledTimes(1);
	});

	test("executes duplicate requestIds once and replies on the request's channel", async () => {
		const request = {
			requestId: "tool-1",
			toolName: "database_tool",
			arguments: { operation: "list_tables" },
		};
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				sseResponse(
					runFrame("run/with spaces"),
					toolRequestFrame("run/with spaces", request),
					toolRequestFrame("run/with spaces", request),
					frame("final", { status: "ok" }),
				),
			)
			.mockResolvedValueOnce(new Response("ack", { status: 200 }));
		vi.stubGlobal("fetch", fetchMock);
		const onToolRequest = vi.fn().mockResolvedValue({
			requestId: "wrong-id",
			approved: true,
			result: { status: "ok" },
		});
		const lifecycle: IAgentDebugEvent[] = [];

		await webGlobalChatStart({
			baseUrl: "https://flow.example",
			body: {},
			onToolRequest,
			onLifecycle: (event) => lifecycle.push(event),
		})(() => undefined);

		expect(onToolRequest).toHaveBeenCalledTimes(1);
		expect(fetchMock).toHaveBeenCalledTimes(2);
		const [deliveryUrl, deliveryInit] = fetchMock.mock.calls[1] as [
			string,
			RequestInit,
		];
		expect(deliveryUrl).toBe(pushUrl("run/with spaces"));
		expect((deliveryInit.headers as Record<string, string>).authorization).toBe(
			"Bearer push-token",
		);
		expect(pushBody(deliveryInit)).toMatchObject({
			channel_id: "run/with spaces",
			request_id: "tool-1",
			kind: "reply",
			value: { requestId: "tool-1", approved: true },
		});
		expect(
			lifecycle.some(
				(event) => event.stage === "duplicate_tool_request_ignored",
			),
		).toBe(true);
		expect(
			lifecycle.some((event) => event.stage === "tool_result_delivered"),
		).toBe(true);
	});

	test("distinguishes handler failures from explicit user denial", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				sseResponse(
					runFrame("run-2"),
					toolRequestFrame("run-2", {
						requestId: "failed-tool",
						toolName: "flowpilot_board",
						arguments: {},
					}),
					toolRequestFrame("run-2", {
						requestId: "denied-tool",
						toolName: "create_app",
						arguments: {},
					}),
					frame("final", { status: "ok" }),
				),
			)
			.mockResolvedValue(new Response(null, { status: 204 }));
		vi.stubGlobal("fetch", fetchMock);
		const lifecycle: IAgentDebugEvent[] = [];

		await webGlobalChatStart({
			baseUrl: "https://flow.example",
			body: {},
			onToolRequest: async (request) => {
				if (request.requestId === "failed-tool")
					throw new Error("handler crashed");
				return {
					requestId: request.requestId,
					approved: false,
					error: "User denied the request.",
				};
			},
			onLifecycle: (event) => lifecycle.push(event),
		})(() => undefined);

		const posted = fetchMock.mock.calls
			.slice(1)
			.map((call) => pushBody(call[1] as RequestInit).value)
			.sort((left, right) =>
				String(left.requestId).localeCompare(String(right.requestId)),
			);
		expect(posted).toEqual([
			{
				requestId: "denied-tool",
				approved: false,
				error: "User denied the request.",
			},
			{
				requestId: "failed-tool",
				approved: true,
				error: "handler crashed",
			},
		]);
		expect(
			lifecycle.find(
				(event) =>
					event.request_id === "failed-tool" &&
					event.stage === "browser_tool_failed",
			)?.status,
		).toBe("error");
		const denial = lifecycle.find(
			(event) =>
				event.request_id === "denied-tool" &&
				event.stage === "browser_tool_denied",
		);
		expect(denial?.status).toBe("denied");
		expect(denial?.error).toBeUndefined();
	});

	test("fails the run when the channel rejects a tool result", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				sseResponse(
					runFrame("run-3"),
					toolRequestFrame("run-3", {
						requestId: "tool-3",
						toolName: "database_tool",
						arguments: {},
					}),
					frame("final", { status: "ok" }),
				),
			)
			.mockResolvedValueOnce(
				new Response("result receiver unavailable", { status: 503 }),
			);
		vi.stubGlobal("fetch", fetchMock);
		const lifecycle: IAgentDebugEvent[] = [];

		await expect(
			webGlobalChatStart({
				baseUrl: "https://flow.example",
				body: {},
				onToolRequest: async (request) => ({
					requestId: request.requestId,
					approved: true,
					result: { status: "ok" },
				}),
				onLifecycle: (event) => lifecycle.push(event),
			})(() => undefined),
		).rejects.toThrow(/503.*result receiver unavailable/);
		expect(
			lifecycle.some((event) => event.stage === "tool_result_delivery_failed"),
		).toBe(true);
	});

	test("falls back to the API push endpoint when the transport fails", async () => {
		const fallbackUrl = "https://flow.example/api/v1/channels/run-fb/push";
		const request = {
			requestId: "tool-fb",
			toolName: "database_tool",
			arguments: {},
		};
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				sseResponse(
					runFrame("run-fb"),
					frame("tool_request", {
						...request,
						channel: {
							channel_id: "run-fb",
							request_id: "tool-fb",
							expires_at: 4_102_444_800,
							transport: {
								type: "http",
								push_url: "https://edge.example/push",
								token: "edge-token",
							},
							fallback: {
								type: "http",
								push_url: fallbackUrl,
								token: "fallback-token",
							},
						} satisfies IChannelHandle,
					}),
					frame("final", { status: "ok" }),
				),
			)
			.mockRejectedValueOnce(new TypeError("edge unreachable"))
			.mockResolvedValueOnce(new Response(null, { status: 204 }));
		vi.stubGlobal("fetch", fetchMock);
		vi.spyOn(console, "warn").mockImplementation(() => undefined);

		await webGlobalChatStart({
			baseUrl: "https://flow.example",
			body: {},
			onToolRequest: async (req) => ({
				requestId: req.requestId,
				approved: true,
				result: {},
			}),
		})(() => undefined);

		expect(fetchMock.mock.calls.map((call) => call[0])).toEqual([
			"https://flow.example/api/v1/ai/global-chat",
			"https://edge.example/push",
			fallbackUrl,
		]);
		expect(
			(fetchMock.mock.calls[2][1] as RequestInit).headers,
		).toMatchObject({ authorization: "Bearer fallback-token" });
	});

	test("bounds a hanging tool-result delivery", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				sseResponse(
					runFrame("run-4"),
					toolRequestFrame("run-4", {
						requestId: "tool-4",
						toolName: "database_tool",
						arguments: {},
					}),
					frame("final", { status: "ok" }),
				),
			)
			.mockImplementationOnce(
				(_url: string, init?: RequestInit) =>
					new Promise<Response>((_resolve, reject) => {
						init?.signal?.addEventListener("abort", () =>
							reject(new DOMException("aborted", "AbortError")),
						);
					}),
			);
		vi.stubGlobal("fetch", fetchMock);

		await expect(
			webGlobalChatStart({
				baseUrl: "https://flow.example",
				body: {},
				toolResultDeliveryTimeoutMs: 5,
				onToolRequest: async (request) => ({
					requestId: request.requestId,
					approved: true,
					result: { status: "ok" },
				}),
			})(() => undefined),
		).rejects.toThrow(/timed out after 5 ms/);
	});

	test("bounds a hanging tool handler and delivers a timeout result", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				sseResponse(
					runFrame("run-handler-timeout"),
					toolRequestFrame("run-handler-timeout", {
						requestId: "hung-tool",
						toolName: "flowpilot_board",
						arguments: {},
					}),
					frame("final", { status: "ok" }),
				),
			)
			.mockResolvedValueOnce(new Response(null, { status: 204 }));
		vi.stubGlobal("fetch", fetchMock);
		const onToolCancel = vi.fn();
		const lifecycle: IAgentDebugEvent[] = [];

		const result = await webGlobalChatStart({
			baseUrl: "https://flow.example",
			body: {},
			toolExecutionTimeoutMs: 5,
			onToolRequest: async () => new Promise(() => undefined),
			onToolCancel,
			onLifecycle: (event) => lifecycle.push(event),
		})(() => undefined);

		expect(result).toEqual({ status: "ok" });
		expect(onToolCancel).toHaveBeenCalledTimes(1);
		const delivered = pushBody(fetchMock.mock.calls[1]?.[1] as RequestInit)
			.value;
		expect(delivered).toMatchObject({
			requestId: "hung-tool",
			approved: true,
		});
		expect(delivered.error).toMatch(/execution timed out after 5 ms/);
		expect(
			lifecycle.some(
				(event) =>
					event.request_id === "hung-tool" &&
					event.stage === "browser_tool_timed_out" &&
					event.status === "timeout",
			),
		).toBe(true);
	});

	test("surfaces malformed protocol frames and missing terminal frames", async () => {
		const malformedLifecycle: IAgentDebugEvent[] = [];
		const malformedFetch = vi
			.fn()
			.mockResolvedValueOnce(
				sseResponse(
					runFrame("run-5"),
					frame("tool_request", {
						toolName: "database_tool",
						arguments: {},
						channel: httpHandle("run-5", "x"),
					}),
					frame("final", { status: "ok" }),
				),
			);
		vi.stubGlobal("fetch", malformedFetch);
		await expect(
			webGlobalChatStart({
				baseUrl: "https://flow.example",
				body: {},
				onLifecycle: (event) => malformedLifecycle.push(event),
			})(() => undefined),
		).rejects.toThrow(/requestId is missing/);
		expect(
			malformedLifecycle.some(
				(event) => event.stage === "malformed_tool_request",
			),
		).toBe(true);

		const channelLessToolFetch = vi
			.fn()
			.mockResolvedValueOnce(
				sseResponse(
					runFrame("run-5b"),
					frame("tool_request", {
						requestId: "tool-5b",
						toolName: "database_tool",
						arguments: {},
					}),
					frame("final", { status: "ok" }),
				),
			);
		vi.stubGlobal("fetch", channelLessToolFetch);
		await expect(
			webGlobalChatStart({
				baseUrl: "https://flow.example",
				body: {},
				onToolRequest: vi.fn(),
			})(() => undefined),
		).rejects.toThrow(/channel is missing/);
		expect(channelLessToolFetch).toHaveBeenCalledTimes(1);

		const channelLessRunLifecycle: IAgentDebugEvent[] = [];
		const channelLessRunFetch = vi
			.fn()
			.mockResolvedValueOnce(
				sseResponse(
					frame("run", { runId: "run-5c" }),
					frame("final", { status: "ok" }),
				),
			);
		vi.stubGlobal("fetch", channelLessRunFetch);
		await expect(
			webGlobalChatStart({
				baseUrl: "https://flow.example",
				body: {},
				onLifecycle: (event) => channelLessRunLifecycle.push(event),
			})(() => undefined),
		).rejects.toThrow(/run frame: channel is missing/);
		expect(
			channelLessRunLifecycle.some(
				(event) => event.stage === "malformed_run_frame",
			),
		).toBe(true);

		const missingRunLifecycle: IAgentDebugEvent[] = [];
		const missingRunFetch = vi
			.fn()
			.mockResolvedValueOnce(sseResponse(frame("final", { status: "ok" })));
		vi.stubGlobal("fetch", missingRunFetch);
		await expect(
			webGlobalChatStart({
				baseUrl: "https://flow.example",
				body: {},
				onLifecycle: (event) => missingRunLifecycle.push(event),
			})(() => undefined),
		).rejects.toThrow(/without a valid run frame/);
		expect(
			missingRunLifecycle.some((event) => event.stage === "missing_run_frame"),
		).toBe(true);

		const missingFinalFetch = vi
			.fn()
			.mockResolvedValueOnce(sseResponse(runFrame("run-6")));
		vi.stubGlobal("fetch", missingFinalFetch);
		await expect(
			webGlobalChatStart({
				baseUrl: "https://flow.example",
				body: {},
			})(() => undefined),
		).rejects.toThrow(/without a final frame/);
	});

	test("maps a client run id onto the server's while the run is live", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				sseResponse(
					runFrame("server-run-1"),
					frame("token", "hi"),
					frame("final", { status: "ok" }),
				),
			);
		vi.stubGlobal("fetch", fetchMock);
		// Before the run frame the client id is all there is to address.
		expect(globalChatTransportRunId("client-run-1")).toBe("client-run-1");
		let liveMapping: string | undefined;

		await webGlobalChatStart({
			baseUrl: "https://flow.example",
			body: {},
			clientRunId: "client-run-1",
		})(() => {
			liveMapping = globalChatTransportRunId("client-run-1");
		});

		// A nested specialist resolves the address mid-run; afterwards it falls back to the client
		// id so a stale mapping can never point at a finished run.
		expect(liveMapping).toBe("server-run-1");
		expect(globalChatTransportRunId("client-run-1")).toBe("client-run-1");
		expect(fetchMock).toHaveBeenCalledTimes(1);
	});

	test("steers and cancels a live run through the run channel", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				sseResponse(
					runFrame("server-run-ctl"),
					frame("token", "hi"),
					frame("final", { status: "ok" }),
				),
			)
			.mockResolvedValue(new Response(null, { status: 204 }));
		vi.stubGlobal("fetch", fetchMock);
		let control: Promise<void> | undefined;

		await webGlobalChatStart({
			baseUrl: "https://flow.example",
			body: {},
			clientRunId: "client-run-ctl",
		})(() => {
			const runControl = getGlobalChatRunControl("client-run-ctl");
			control = runControl
				?.steer("focus on tests")
				.then(() => runControl.cancel());
		});
		await control;

		expect(fetchMock.mock.calls.slice(1).map((call) => call[0])).toEqual([
			pushUrl("server-run-ctl"),
			pushUrl("server-run-ctl"),
		]);
		const bodies = fetchMock.mock.calls
			.slice(1)
			.map((call) => pushBody(call[1] as RequestInit));
		expect(bodies).toEqual([
			{ channel_id: "server-run-ctl", kind: "inbound", value: "focus on tests" },
			{ channel_id: "server-run-ctl", kind: "cancel", value: null },
		]);
		expect(getGlobalChatRunControl("client-run-ctl")).toBeUndefined();
	});

	test("dispatches a nested specialist tool request on the frame's channel", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(new Response(null, { status: 204 }));
		vi.stubGlobal("fetch", fetchMock);
		const onToolRequest = vi.fn().mockResolvedValue({
			requestId: "ignored-id",
			approved: true,
			result: { status: "ok", tables: [] },
		});

		await dispatchSpecialistToolRequest({
			data: JSON.stringify({
				requestId: "tool-9",
				toolName: "database_tool",
				arguments: { operation: "list_tables" },
				channel: httpHandle("server-run-2", "tool-9"),
			}),
			onToolRequest,
		});

		expect(onToolRequest).toHaveBeenCalledTimes(1);
		const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
		expect(url).toBe(pushUrl("server-run-2"));
		expect((init.headers as Record<string, string>).authorization).toBe(
			"Bearer push-token",
		);
		// The specialist is blocked on the id it sent, never on one a handler made up.
		expect(pushBody(init)).toMatchObject({
			channel_id: "server-run-2",
			request_id: "tool-9",
			kind: "reply",
			value: { requestId: "tool-9", approved: true },
		});
	});

	test("rejects a malformed specialist tool frame instead of posting a guess", async () => {
		const fetchMock = vi.fn();
		vi.stubGlobal("fetch", fetchMock);

		await expect(
			dispatchSpecialistToolRequest({
				data: JSON.stringify({ toolName: "database_tool" }),
				onToolRequest: vi.fn(),
			}),
		).rejects.toThrow(/requestId is missing/);
		expect(fetchMock).not.toHaveBeenCalled();
	});
});
