import type { IAgentDebugEvent } from "@flow-like/flow-like-ui/state/global-chat/agent-debug-report";
import {
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
					frame("run", { runId: "run-1" }),
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

	test("executes duplicate requestIds once and forces the original response id", async () => {
		const request = {
			requestId: "tool-1",
			toolName: "database_tool",
			arguments: { operation: "list_tables" },
		};
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				sseResponse(
					frame("run", { runId: "run/with spaces" }),
					frame("tool_request", request),
					frame("tool_request", request),
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
		expect(deliveryUrl).toContain("run%2Fwith%20spaces/tool-result");
		expect(JSON.parse(String(deliveryInit.body))).toMatchObject({
			requestId: "tool-1",
			approved: true,
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
					frame("run", { runId: "run-2" }),
					frame("tool_request", {
						requestId: "failed-tool",
						toolName: "flowpilot_board",
						arguments: {},
					}),
					frame("tool_request", {
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
			.map((call) => JSON.parse(String((call[1] as RequestInit).body)))
			.sort((left, right) => left.requestId.localeCompare(right.requestId));
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

	test("fails the run when the server rejects a tool result", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				sseResponse(
					frame("run", { runId: "run-3" }),
					frame("tool_request", {
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

	test("bounds a hanging tool-result delivery", async () => {
		const fetchMock = vi
			.fn()
			.mockResolvedValueOnce(
				sseResponse(
					frame("run", { runId: "run-4" }),
					frame("tool_request", {
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
					frame("run", { runId: "run-handler-timeout" }),
					frame("tool_request", {
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
		const delivered = JSON.parse(
			String((fetchMock.mock.calls[1]?.[1] as RequestInit).body),
		);
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
					frame("run", { runId: "run-5" }),
					frame("tool_request", { toolName: "database_tool", arguments: {} }),
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
			.mockResolvedValueOnce(sseResponse(frame("run", { runId: "run-6" })));
		vi.stubGlobal("fetch", missingFinalFetch);
		await expect(
			webGlobalChatStart({
				baseUrl: "https://flow.example",
				body: {},
			})(() => undefined),
		).rejects.toThrow(/without a final frame/);
	});
});
