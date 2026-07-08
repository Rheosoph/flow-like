// Browser transport for the global-chat streaming engine (`driveGlobalChatStream`). The desktop
// drives a run over a Tauri `Channel` + a separate Tauri event for tool requests; in the browser the
// same run is one HTTP request whose SSE response multiplexes everything:
//
//   event: run           data: { "runId": "..." }                      → address for tool results
//   event: token         data: <raw FlowPilot stream chunk>            → forwarded to the parser
//   event: tool_request  data: { requestId, toolName, arguments, ... } → executed here, result POSTed back
//   event: final         data: <UnifiedCopilotResponse JSON>           → resolves the run
//   event: error         data: { "error": "..." }                      → throws
//
// `token`/`tool_request`/`run`/`final` mirror the exact payload shapes the desktop uses, so the
// shared stream parser and the frontend tool bridge behave identically on both transports.

import type {
	FrontendToolRequest,
	FrontendToolResponse,
} from "../../components/global-chat/global-tool-bridge";

// The desktop and browser tool contracts are identical; reuse the canonical bridge types so a single
// executor (see `global-chat-tool-registry`) satisfies both transports.
export type WebToolRequest = FrontendToolRequest;
export type WebToolResponse = FrontendToolResponse;

export interface WebGlobalChatOptions {
	/** API origin (scheme + host, no `/api/v1`), e.g. from `getApiOrigin()`. */
	baseUrl: string;
	/** The user's access token (OpenID). Required — hosted Bit models are billed against it. */
	token?: string;
	/** POST body for `/ai/global-chat` (userPrompt, history, modelId, embeddingModelId, …). */
	body: Record<string, unknown>;
	/**
	 * Execute one browser tool and resolve with its result. Wire this to the same handlers the
	 * desktop tool bridge uses (navigate, open_app_chat, flowpilot_board, ask_user, …). When omitted
	 * every tool request is auto-denied, which degrades to a text-only assistant.
	 */
	onToolRequest?: (request: WebToolRequest) => Promise<WebToolResponse>;
}

/**
 * Build a `start(onChunk)` transport for {@link driveGlobalChatStream} that talks to the browser API.
 * Forwards `token` frames to `onChunk`, dispatches `tool_request` frames to `onToolRequest` and POSTs
 * the result back keyed by the run id, and resolves with the final `UnifiedCopilotResponse`.
 */
export function webGlobalChatStart(options: WebGlobalChatOptions) {
	return async (onChunk: (chunk: string) => void): Promise<unknown> => {
		const { baseUrl, token, body, onToolRequest } = options;
		const authHeaders: Record<string, string> = token
			? { authorization: `Bearer ${token}` }
			: {};

		const response = await fetch(`${baseUrl}/api/v1/ai/global-chat`, {
			method: "POST",
			headers: {
				"content-type": "application/json",
				accept: "text/event-stream",
				...authHeaders,
			},
			body: JSON.stringify(body),
		});

		if (!response.ok || !response.body) {
			const text = await response.text().catch(() => "");
			throw new Error(
				`FlowPilot request failed (${response.status}): ${text.slice(0, 300)}`,
			);
		}

		let runId: string | undefined;
		let finalResult: unknown;
		let streamError: Error | undefined;

		const handleEvent = (eventName: string, data: string) => {
			switch (eventName) {
				case "run":
					try {
						runId = (JSON.parse(data) as { runId?: string }).runId;
					} catch {
						// keep runId undefined; tool results simply won't be routable
					}
					return;
				case "token":
					onChunk(data);
					return;
				case "tool_request":
					dispatchToolRequest(baseUrl, authHeaders, runId, data, onToolRequest);
					return;
				case "final":
					try {
						finalResult = JSON.parse(data);
					} catch {
						finalResult = undefined;
					}
					return;
				case "error":
					try {
						streamError = new Error(
							(JSON.parse(data) as { error?: string }).error ??
								"FlowPilot error",
						);
					} catch {
						streamError = new Error("FlowPilot error");
					}
			}
		};

		const reader = response.body.getReader();
		const decoder = new TextDecoder();
		let buffer = "";
		try {
			while (true) {
				const { value, done } = await reader.read();
				if (done) break;
				buffer += decoder.decode(value, { stream: true });
				// SSE frames are separated by a blank line.
				let sep = buffer.indexOf("\n\n");
				while (sep !== -1) {
					const frame = buffer.slice(0, sep);
					buffer = buffer.slice(sep + 2);
					const parsed = parseSseFrame(frame);
					if (parsed) handleEvent(parsed.event, parsed.data);
					sep = buffer.indexOf("\n\n");
				}
			}
		} finally {
			reader.releaseLock();
		}

		if (streamError) throw streamError;
		return finalResult;
	};
}

/** Parse one raw SSE frame into `{ event, data }`. Returns null for keep-alive/comment-only frames. */
function parseSseFrame(frame: string): { event: string; data: string } | null {
	let event = "message";
	const dataLines: string[] = [];
	for (const line of frame.split("\n")) {
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

/** Run a browser tool and POST its result to `/ai/global-chat/{runId}/tool-result`. */
function dispatchToolRequest(
	baseUrl: string,
	authHeaders: Record<string, string>,
	runId: string | undefined,
	data: string,
	onToolRequest: WebGlobalChatOptions["onToolRequest"],
) {
	let request: WebToolRequest;
	try {
		request = JSON.parse(data) as WebToolRequest;
	} catch {
		return;
	}
	if (!runId) return;

	// Execute off the read loop so streaming keeps flowing while the tool (and any approval dialog)
	// runs. The server call blocks on our POST, so a slow tool holds only the server-side future.
	void (async () => {
		let result: WebToolResponse;
		if (onToolRequest) {
			try {
				result = await onToolRequest(request);
			} catch (error) {
				result = {
					requestId: request.requestId,
					approved: false,
					error: error instanceof Error ? error.message : String(error),
				};
			}
		} else {
			result = {
				requestId: request.requestId,
				approved: false,
				error: "No tool handler is wired for this browser session.",
			};
		}

		try {
			await fetch(
				`${baseUrl}/api/v1/ai/global-chat/${encodeURIComponent(runId)}/tool-result`,
				{
					method: "POST",
					headers: { "content-type": "application/json", ...authHeaders },
					body: JSON.stringify(result),
				},
			);
		} catch {
			// If the result never lands, the server-side tool call times out and the loop continues.
		}
	})();
}
