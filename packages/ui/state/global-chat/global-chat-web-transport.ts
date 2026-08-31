// Browser transport for the global-chat streaming engine (`driveGlobalChatStream`). The desktop
// drives a run over a Tauri `Channel` + a separate Tauri event for tool requests; in the browser the
// same run is one HTTP request whose SSE response multiplexes everything:
//
//   event: run           data: { "runId": "...", "channel": IChannelHandle }  → run-level channel (cancel / steer)
//   event: token         data: <raw FlowPilot stream chunk>                   → forwarded to the parser
//   event: tool_request  data: { requestId, toolName, arguments, channel, … } → executed here, result replied on its channel
//   event: final         data: <UnifiedCopilotResponse JSON>                  → resolves the run
//   event: error         data: { "error": "..." }                             → throws
//
// Nothing goes back up this stream: every request carries a channel handle naming the transport
// the waiter listens on (see `lib/channel`), with the API push endpoint as fallback.
// `token`/`tool_request`/`run`/`final` mirror the exact payload shapes the desktop uses, so the
// shared stream parser and the frontend tool bridge behave identically on both transports.

import type {
	FrontendToolRequest,
	FrontendToolResponse,
} from "../../components/global-chat/global-tool-bridge";
import { apiResponseError } from "../../lib/api-error";
import {
	cancelChannel,
	isChannelHandle,
	replyToChannel,
	steerChannel,
} from "../../lib/channel";
import type { IChannelHandle } from "../../lib/schema/channel";
import type { IAgentDebugEvent } from "./agent-debug-report";
import {
	registerGlobalChatRunControl,
	registerGlobalChatTransportRunId,
	unregisterGlobalChatRunControl,
	unregisterGlobalChatTransportRunId,
} from "./global-chat-run-control";

// The desktop and browser tool contracts are identical; reuse the canonical bridge types so a single
// executor (see `global-chat-tool-registry`) satisfies both transports. On the wire the channel is
// mandatory — a request without one cannot be answered.
export type WebToolRequest = FrontendToolRequest & { channel: IChannelHandle };
export type WebToolResponse = FrontendToolResponse;

const DEFAULT_TOOL_RESULT_DELIVERY_TIMEOUT_MS = 30_000;
const DEFAULT_TOOL_EXECUTION_TIMEOUT_MS = 10 * 60_000;

export interface ToolDispatchOptions {
	/**
	 * Execute one browser tool and resolve with its result. Wire this to the same handlers the
	 * desktop tool bridge uses (navigate, open_app_chat, flowpilot_board, ask_user, …). When omitted
	 * the tool returns an infrastructure error to the model; it is not misreported as a user denial.
	 */
	onToolRequest?: (request: WebToolRequest) => Promise<WebToolResponse>;
	/** Best-effort cancellation hook for handlers that own abortable/native work. */
	onToolCancel?: (
		request: WebToolRequest,
		reason: string,
	) => void | Promise<void>;
	/**
	 * Optional hook for folding browser transport/tool-delivery milestones into the active message's
	 * persisted debug report. The callback is observational: exceptions never break the run.
	 */
	onLifecycle?: (event: IAgentDebugEvent) => void;
	/** Maximum time allowed for delivering a result / control push (tool execution has its own deadline). */
	toolResultDeliveryTimeoutMs?: number;
	/** Hard upper bound for a browser tool handler, also applied when the server omitted a deadline. */
	toolExecutionTimeoutMs?: number;
}

export interface WebGlobalChatOptions extends ToolDispatchOptions {
	/** API origin (scheme + host, no `/api/v1`), e.g. from `getApiOrigin()`. */
	baseUrl: string;
	/** The user's access token (OpenID). Required — hosted Bit models are billed against it. */
	token?: string;
	/** POST body for `/ai/global-chat` (userPrompt, history, modelId, embeddingModelId, …). */
	body: Record<string, unknown>;
	/**
	 * The run id the FRONTEND minted (the assistant message id). The server mints its own and hands
	 * it back in the `run` frame; only this transport sees both. It uses the client id to register
	 * the run's cancel/steer control and to tag outgoing tool requests, so the tool bridge can route
	 * a tool's output back to the right reply when several turns stream at once.
	 */
	clientRunId?: string;
}

function errorMessage(error: unknown) {
	return error instanceof Error ? error.message : String(error);
}

function parentRequestId(request: WebToolRequest) {
	return (
		request.parentRequestId ??
		request.context?.parentRequestId ??
		request.context?.parent_request_id
	);
}

function deliveryTimeout(options: ToolDispatchOptions) {
	const configured = options.toolResultDeliveryTimeoutMs;
	return typeof configured === "number" && Number.isFinite(configured)
		? Math.max(1, configured)
		: DEFAULT_TOOL_RESULT_DELIVERY_TIMEOUT_MS;
}

function executionTimeout(
	options: ToolDispatchOptions,
	request: WebToolRequest,
) {
	const configured = options.toolExecutionTimeoutMs;
	const explicitCap =
		typeof configured === "number" && Number.isFinite(configured)
			? Math.max(1, configured)
			: undefined;
	const requestDeadline = request.deadlineAtMs ?? request.deadline_at_ms;
	// The backend deadline is authoritative. A delegated board run earns wall clock by proving
	// progress and can legitimately last hours, so a generic client-side default must not cut it
	// short — that default only covers requests arriving without a deadline. An explicitly
	// configured cap still applies, mirroring how the desktop bridge derives its deadline.
	if (typeof requestDeadline === "number" && Number.isFinite(requestDeadline)) {
		const remaining = Math.max(1, requestDeadline - Date.now());
		return explicitCap === undefined
			? remaining
			: Math.min(explicitCap, remaining);
	}
	return explicitCap ?? DEFAULT_TOOL_EXECUTION_TIMEOUT_MS;
}

function emitLifecycle(options: ToolDispatchOptions, event: IAgentDebugEvent) {
	try {
		options.onLifecycle?.(event);
	} catch {
		// Debug reporting must never become a new failure mode for the assistant run.
	}
}

function bridgeEvent(
	id: string,
	stage: string,
	status: string,
	fields: Partial<IAgentDebugEvent> = {},
): IAgentDebugEvent {
	return {
		id,
		kind: "bridge",
		stage,
		status,
		timestamp_ms: Date.now(),
		...fields,
	};
}

/**
 * Bound one channel push by the delivery timeout. Pushes must fail loudly: a stuck result leaves
 * the server blocked on the request, and the composer restores a steering message that never landed.
 */
async function withDeliveryTimeout<T>(
	options: ToolDispatchOptions,
	describe: string,
	run: (signal: AbortSignal) => Promise<T>,
): Promise<T> {
	const timeoutMs = deliveryTimeout(options);
	const controller = new AbortController();
	const timeoutId = setTimeout(() => controller.abort(), timeoutMs);
	try {
		return await run(controller.signal);
	} catch (error) {
		if (controller.signal.aborted) {
			throw new Error(`${describe} timed out after ${timeoutMs} ms.`);
		}
		throw new Error(`${describe} failed: ${errorMessage(error)}`);
	} finally {
		clearTimeout(timeoutId);
	}
}

function parseRunFrame(data: string): {
	runId: string;
	channel: IChannelHandle;
} {
	let value: unknown;
	try {
		value = JSON.parse(data);
	} catch {
		throw new Error("Malformed run frame: payload is not valid JSON.");
	}
	const record = (value ?? {}) as { runId?: unknown; channel?: unknown };
	if (typeof record.runId !== "string" || !record.runId.trim()) {
		throw new Error("Malformed run frame: runId is missing.");
	}
	if (!isChannelHandle(record.channel)) {
		throw new Error("Malformed run frame: channel is missing.");
	}
	return { runId: record.runId, channel: record.channel };
}

function parseToolRequest(data: string): WebToolRequest {
	let value: unknown;
	try {
		value = JSON.parse(data);
	} catch {
		throw new Error("Malformed tool_request frame: payload is not valid JSON.");
	}
	if (!value || typeof value !== "object" || Array.isArray(value)) {
		throw new Error("Malformed tool_request frame: payload must be an object.");
	}
	const record = value as Partial<WebToolRequest>;
	if (typeof record.requestId !== "string" || !record.requestId.trim()) {
		throw new Error("Malformed tool_request frame: requestId is missing.");
	}
	if (typeof record.toolName !== "string" || !record.toolName.trim()) {
		throw new Error("Malformed tool_request frame: toolName is missing.");
	}
	if (
		record.arguments !== undefined &&
		(!record.arguments ||
			typeof record.arguments !== "object" ||
			Array.isArray(record.arguments))
	) {
		throw new Error(
			"Malformed tool_request frame: arguments must be an object.",
		);
	}
	if (!isChannelHandle(record.channel)) {
		throw new Error("Malformed tool_request frame: channel is missing.");
	}
	return {
		...(record as WebToolRequest),
		requestId: record.requestId,
		toolName: record.toolName,
		arguments: record.arguments ?? {},
		channel: record.channel,
	};
}

async function executeBrowserTool(
	options: ToolDispatchOptions,
	request: WebToolRequest,
): Promise<WebToolResponse> {
	const id = `web:${request.requestId}:handler`;
	const startedAt = Date.now();
	emitLifecycle(
		options,
		bridgeEvent(id, "browser_tool_started", "progress", {
			request_id: request.requestId,
			parent_request_id: parentRequestId(request),
			name: request.toolName,
			started_at_ms: startedAt,
		}),
	);

	let result: WebToolResponse;
	let handlerTimedOut = false;
	if (!options.onToolRequest) {
		result = {
			requestId: request.requestId,
			approved: true,
			error: "No tool handler is wired for this browser session.",
		};
	} else {
		const timeoutMs = executionTimeout(options, request);
		const deadlineAtMs = Date.now() + timeoutMs;
		const handlerRequest: WebToolRequest = {
			...request,
			deadlineAtMs: Math.min(
				request.deadlineAtMs ??
					request.deadline_at_ms ??
					Number.POSITIVE_INFINITY,
				deadlineAtMs,
			),
		};
		let timeoutId: ReturnType<typeof setTimeout> | undefined;
		const handler = Promise.resolve()
			.then(() => options.onToolRequest?.(handlerRequest))
			.then((response) => {
				if (!response || typeof response.approved !== "boolean") {
					throw new Error("Browser tool handler returned an invalid response.");
				}
				// Never trust a handler-provided id: the server is blocked on the original request id.
				return { ...response, requestId: request.requestId };
			})
			.catch(
				(error): WebToolResponse => ({
					requestId: request.requestId,
					approved: true,
					error: errorMessage(error),
				}),
			);
		const timeout = new Promise<WebToolResponse>((resolve) => {
			timeoutId = setTimeout(() => {
				handlerTimedOut = true;
				const reason = `Browser tool execution timed out after ${timeoutMs} ms for request '${request.requestId}'.`;
				try {
					void Promise.resolve(
						options.onToolCancel?.(handlerRequest, reason),
					).catch(() => undefined);
				} catch {
					// Cancellation is best-effort; the timeout response must still unblock the server.
				}
				resolve({
					requestId: request.requestId,
					approved: true,
					error: reason,
				});
			}, timeoutMs);
		});
		result = await Promise.race([handler, timeout]);
		if (timeoutId !== undefined) clearTimeout(timeoutId);
	}

	const endedAt = Date.now();
	const denied = !result.approved;
	const failed = !denied && Boolean(result.error);
	const timedOut = handlerTimedOut;
	const handlerStage = timedOut
		? "browser_tool_timed_out"
		: denied
			? "browser_tool_denied"
			: failed
				? "browser_tool_failed"
				: "browser_tool_completed";
	const handlerStatus = timedOut
		? "timeout"
		: denied
			? "denied"
			: failed
				? "error"
				: "done";
	emitLifecycle(
		options,
		bridgeEvent(id, handlerStage, handlerStatus, {
			request_id: request.requestId,
			parent_request_id: parentRequestId(request),
			name: request.toolName,
			ended_at_ms: endedAt,
			error: failed ? result.error : undefined,
			result_summary: denied
				? result.error || "The user denied the browser tool request."
				: failed
					? "Browser tool handler failed."
					: "Browser tool completed.",
		}),
	);
	return result;
}

async function deliverToolResult(
	options: ToolDispatchOptions,
	request: WebToolRequest,
	result: WebToolResponse,
) {
	const id = `web:${request.requestId}:delivery`;
	const startedAt = Date.now();
	emitLifecycle(
		options,
		bridgeEvent(id, "tool_result_delivery_started", "progress", {
			request_id: request.requestId,
			parent_request_id: parentRequestId(request),
			name: request.toolName,
			started_at_ms: startedAt,
		}),
	);

	try {
		await withDeliveryTimeout(
			options,
			`Tool result delivery for request '${request.requestId}'`,
			(signal) =>
				replyToChannel(
					request.channel,
					{ ...result, requestId: request.requestId },
					{ signal },
				),
		);
		emitLifecycle(
			options,
			bridgeEvent(id, "tool_result_delivered", "done", {
				request_id: request.requestId,
				parent_request_id: parentRequestId(request),
				name: request.toolName,
				ended_at_ms: Date.now(),
				result_summary: `Tool result delivered over ${request.channel.transport.type}.`,
			}),
		);
	} catch (error) {
		emitLifecycle(
			options,
			bridgeEvent(id, "tool_result_delivery_failed", "error", {
				request_id: request.requestId,
				parent_request_id: parentRequestId(request),
				name: request.toolName,
				ended_at_ms: Date.now(),
				error: errorMessage(error),
			}),
		);
		throw error;
	}
}

async function dispatchToolRequest(
	options: ToolDispatchOptions,
	request: WebToolRequest,
) {
	const result = await executeBrowserTool(options, request);
	await deliverToolResult(options, request, result);
}

/**
 * Execute one `tool_request` frame that arrived on a nested specialist's own SSE stream
 * (`/ai/copilot/chat` with a Data Studio / Scout scope) and reply on the channel it carries.
 *
 * A specialist rides the owning chat run, so its requests carry that run's channel — only the
 * stream the request travelled down differs from the orchestrator's own tools.
 */
export async function dispatchSpecialistToolRequest(params: {
	data: string;
	onToolRequest: (request: WebToolRequest) => Promise<WebToolResponse>;
	onToolCancel?: ToolDispatchOptions["onToolCancel"];
}): Promise<void> {
	await dispatchToolRequest(
		{ onToolRequest: params.onToolRequest, onToolCancel: params.onToolCancel },
		parseToolRequest(params.data),
	);
}

/**
 * Build a `start(onChunk)` transport for {@link driveGlobalChatStream} that talks to the browser API.
 * Forwards `token` frames to `onChunk`, dispatches `tool_request` frames to `onToolRequest` and
 * replies on each request's channel, and resolves with the final `UnifiedCopilotResponse`.
 */
export function webGlobalChatStart(options: WebGlobalChatOptions) {
	return async (onChunk: (chunk: string) => void): Promise<unknown> => {
		const { baseUrl, token, body } = options;
		const authHeaders: Record<string, string> = token
			? { authorization: `Bearer ${token}` }
			: {};
		const transportId = "web:global-chat:transport";
		const transportStartedAt = Date.now();
		emitLifecycle(
			options,
			bridgeEvent(transportId, "web_transport_started", "progress", {
				started_at_ms: transportStartedAt,
			}),
		);

		// Stopping a browser run has two halves: abort the SSE request so the UI stops immediately,
		// and tell the server to cancel the generation so it does not keep burning tokens.
		//
		// The control is registered BEFORE the request goes out, closing over a run channel that is
		// only filled in when the server's `run` frame lands. Stopping in that window still aborts
		// the stream locally; steering in it is refused rather than silently dropped.
		const streamAbort = new AbortController();
		let runChannel: IChannelHandle | undefined;
		if (options.clientRunId) {
			registerGlobalChatRunControl(options.clientRunId, {
				cancel: async () => {
					streamAbort.abort();
					const channel = runChannel;
					if (!channel) return;
					await withDeliveryTimeout(options, "FlowPilot cancel", (signal) =>
						cancelChannel(channel, { signal }),
					);
				},
				steer: async (content) => {
					const channel = runChannel;
					if (!channel) {
						throw new Error("The run has not started yet.");
					}
					await withDeliveryTimeout(options, "FlowPilot steer", (signal) =>
						steerChannel(channel, content, { signal }),
					);
				},
			});
		}
		try {
			const response = await fetch(`${baseUrl}/api/v1/ai/global-chat`, {
				method: "POST",
				headers: {
					"content-type": "application/json",
					accept: "text/event-stream",
					...authHeaders,
				},
				body: JSON.stringify(body),
				signal: streamAbort.signal,
			});

			if (!response.ok) {
				// Keep the API envelope (code, message, incident id) on the error object: the chat
				// renders a classified failure card from it, not from a stringified body.
				const text = await response.text().catch(() => "");
				throw apiResponseError(response, text, "/api/v1/ai/global-chat");
			}
			if (!response.body) {
				throw new Error("FlowPilot accepted the request but sent no stream.");
			}

			let runId: string | undefined;
			let finalResult: unknown;
			let sawFinal = false;
			let transportError: Error | undefined;
			let protocolSequence = 0;
			const seenToolRequestIds = new Set<string>();
			const outstandingDispatches = new Set<Promise<void>>();
			const reader = response.body.getReader();
			let cancellation: Promise<unknown> | undefined;

			const cancelStream = (error: Error) => {
				if (!transportError) transportError = error;
				if (!cancellation) {
					cancellation = reader.cancel(error.message).catch(() => undefined);
				}
			};
			const protocolFailure = (stage: string, message: string) => {
				const error = new Error(message);
				emitLifecycle(
					options,
					bridgeEvent(`web:protocol:${protocolSequence++}`, stage, "error", {
						error: message,
					}),
				);
				cancelStream(error);
			};
			const trackDispatch = (promise: Promise<void>) => {
				const tracked = promise.catch((error) =>
					cancelStream(
						error instanceof Error ? error : new Error(String(error)),
					),
				);
				outstandingDispatches.add(tracked);
				void tracked.finally(() => outstandingDispatches.delete(tracked));
			};

			const handleEvent = (eventName: string, data: string) => {
				if (transportError) return;
				switch (eventName) {
					case "run": {
						let parsed: { runId: string; channel: IChannelHandle };
						try {
							parsed = parseRunFrame(data);
						} catch (error) {
							protocolFailure("malformed_run_frame", errorMessage(error));
							return;
						}
						if (runId && runId !== parsed.runId) {
							protocolFailure(
								"conflicting_run_frame",
								"Conflicting run frame received for an active FlowPilot stream.",
							);
							return;
						}
						runId = parsed.runId;
						// The run is addressable from here on — the control registered above starts
						// reaching the server instead of only aborting locally.
						runChannel = parsed.channel;
						// Nested specialist runs are separate HTTP requests that still ride THIS run,
						// so they need the server's id too.
						if (options.clientRunId) {
							registerGlobalChatTransportRunId(
								options.clientRunId,
								parsed.runId,
							);
						}
						emitLifecycle(
							options,
							bridgeEvent("web:protocol:run", "run_frame_received", "done", {
								summary: `Run channel transport: ${parsed.channel.transport.type}.`,
							}),
						);
						return;
					}
					case "token":
						try {
							onChunk(data);
						} catch (error) {
							protocolFailure(
								"token_handler_failed",
								`FlowPilot token handler failed: ${errorMessage(error)}`,
							);
						}
						return;
					case "tool_request": {
						if (!runId) {
							protocolFailure(
								"tool_request_without_run",
								"Received a tool_request frame before a valid run frame.",
							);
							return;
						}
						let request: WebToolRequest;
						try {
							request = parseToolRequest(data);
						} catch (error) {
							protocolFailure("malformed_tool_request", errorMessage(error));
							return;
						}
						if (seenToolRequestIds.has(request.requestId)) {
							emitLifecycle(
								options,
								bridgeEvent(
									`web:${request.requestId}:duplicate`,
									"duplicate_tool_request_ignored",
									"cancelled",
									{
										request_id: request.requestId,
										name: request.toolName,
										summary:
											"Duplicate requestId ignored to prevent a repeated side effect.",
									},
								),
							);
							return;
						}
						seenToolRequestIds.add(request.requestId);
						// The server knows only its own run id, so stamp ours on the way through:
						// the bridge routes every store write this tool performs by it.
						if (options.clientRunId) {
							request = {
								...request,
								context: {
									...request.context,
									runId: options.clientRunId,
								},
							};
						}
						emitLifecycle(
							options,
							bridgeEvent(
								`web:${request.requestId}:request`,
								"tool_request_received",
								"done",
								{
									request_id: request.requestId,
									parent_request_id: parentRequestId(request),
									name: request.toolName,
								},
							),
						);
						trackDispatch(dispatchToolRequest(options, request));
						return;
					}
					case "final":
						if (sawFinal) {
							protocolFailure(
								"duplicate_final_frame",
								"Received more than one final frame for the FlowPilot run.",
							);
							return;
						}
						try {
							finalResult = JSON.parse(data);
							sawFinal = true;
							emitLifecycle(
								options,
								bridgeEvent(
									"web:protocol:final",
									"final_frame_received",
									"done",
								),
							);
						} catch {
							protocolFailure(
								"malformed_final_frame",
								"Malformed final frame: payload is not valid JSON.",
							);
						}
						return;
					case "error": {
						let message = "FlowPilot error";
						try {
							const parsed = JSON.parse(data) as { error?: unknown };
							if (typeof parsed.error === "string" && parsed.error)
								message = parsed.error;
						} catch {
							// Keep the bounded generic error rather than persisting malformed raw data.
						}
						emitLifecycle(
							options,
							bridgeEvent("web:protocol:error", "server_error_frame", "error", {
								error: message,
							}),
						);
						cancelStream(new Error(message));
					}
				}
			};

			const decoder = new TextDecoder();
			let buffer = "";
			const drainCompleteFrames = () => {
				while (!transportError) {
					const separator = /\r?\n\r?\n/.exec(buffer);
					if (!separator || separator.index === undefined) return;
					const frame = buffer.slice(0, separator.index);
					buffer = buffer.slice(separator.index + separator[0].length);
					const parsed = parseSseFrame(frame);
					if (parsed) handleEvent(parsed.event, parsed.data);
				}
			};

			try {
				while (!transportError) {
					const { value, done } = await reader.read();
					if (done) break;
					buffer += decoder.decode(value, { stream: true });
					drainCompleteFrames();
				}
				buffer += decoder.decode();
				drainCompleteFrames();
				// Servers are allowed to close immediately after the final frame without a trailing blank line.
				if (!transportError && buffer.trim()) {
					const parsed = parseSseFrame(buffer);
					buffer = "";
					if (parsed) handleEvent(parsed.event, parsed.data);
				}
			} finally {
				if (cancellation) await cancellation;
				reader.releaseLock();
			}

			// A final frame should normally follow delivery, but awaiting the tracked tasks closes a race
			// with servers that optimistically finish their SSE stream before consuming the reply.
			await Promise.all([...outstandingDispatches]);
			if (transportError) throw transportError;
			if (!runId) {
				const error = new Error(
					"FlowPilot stream ended without a valid run frame.",
				);
				emitLifecycle(
					options,
					bridgeEvent(
						"web:protocol:missing-run",
						"missing_run_frame",
						"error",
						{
							error: error.message,
						},
					),
				);
				throw error;
			}
			if (!sawFinal) {
				const error = new Error(
					"FlowPilot stream ended without a final frame.",
				);
				emitLifecycle(
					options,
					bridgeEvent(
						"web:protocol:missing-final",
						"missing_final_frame",
						"error",
						{
							error: error.message,
						},
					),
				);
				throw error;
			}

			emitLifecycle(
				options,
				bridgeEvent(transportId, "web_transport_completed", "done", {
					ended_at_ms: Date.now(),
					result_summary: "Browser FlowPilot stream completed.",
				}),
			);
			return finalResult;
		} catch (error) {
			const failure = error instanceof Error ? error : new Error(String(error));
			emitLifecycle(
				options,
				bridgeEvent(transportId, "web_transport_failed", "error", {
					ended_at_ms: Date.now(),
					error: failure.message,
				}),
			);
			throw failure;
		} finally {
			if (options.clientRunId) {
				unregisterGlobalChatRunControl(options.clientRunId);
				unregisterGlobalChatTransportRunId(options.clientRunId);
			}
		}
	};
}

/** Parse one raw SSE frame into `{ event, data }`. Returns null for keep-alive/comment-only frames. */
export function parseSseFrame(
	frame: string,
): { event: string; data: string } | null {
	let event = "message";
	const dataLines: string[] = [];
	for (const line of frame.split(/\r?\n/)) {
		if (line.startsWith(":")) continue; // comment / keep-alive
		if (line.startsWith("event:")) {
			event = line.slice(6).trim();
		} else if (line.startsWith("data:")) {
			// A leading single space after the colon is part of the SSE framing, not the data.
			dataLines.push(line.slice(5).replace(/^ /, ""));
		}
	}
	if (dataLines.length === 0) return null;
	return { event, data: dataLines.join("\n") };
}
