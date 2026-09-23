import type {
	IAIState,
	IHistoryMessage,
	IResponse,
	IResponseChunk,
} from "@flow-like/flow-like-ui";
import { type WebBackendRef, getApiBaseUrl } from "./api-utils";

const parseDataLines = (lines: readonly string[]): IResponseChunk[] =>
	lines.flatMap((line) => {
		if (!line.startsWith("data: ")) return [];
		try {
			return [JSON.parse(line.slice(6)) as IResponseChunk];
		} catch {
			return [];
		}
	});

export class WebAIState implements IAIState {
	constructor(private readonly backend: WebBackendRef) {}

	async streamChatComplete(
		messages: IHistoryMessage[],
		appId?: string,
	): Promise<ReadableStream<IResponseChunk[]>> {
		const baseUrl = getApiBaseUrl();
		const url = `${baseUrl}/api/v1/ai/copilot/chat`;

		const headers: HeadersInit = {
			"Content-Type": "application/json",
			Accept: "text/event-stream",
		};
		if (this.backend.auth?.user?.access_token) {
			headers["Authorization"] =
				`Bearer ${this.backend.auth.user.access_token}`;
		}

		const response = await fetch(url, {
			method: "POST",
			headers,
			body: JSON.stringify({ messages, app_id: appId }),
		});

		if (!response.ok) {
			throw new Error(`AI stream failed: ${response.status}`);
		}

		if (!response.body) {
			throw new Error("No response body for streaming");
		}

		const reader = response.body.getReader();
		const decoder = new TextDecoder();
		// A `data:` line can span two network reads; the tail waits for the next one.
		let pending = "";

		return new ReadableStream<IResponseChunk[]>({
			// A pull that enqueues nothing is not repeated, so read until a chunk or the end.
			async pull(controller) {
				for (;;) {
					const { done, value } = await reader.read();
					pending += decoder.decode(value, { stream: !done });
					const lines = pending.split("\n");
					pending = done ? "" : (lines.pop() ?? "");
					const chunks = parseDataLines(lines);

					if (chunks.length > 0) controller.enqueue(chunks);
					if (done) {
						controller.close();
						return;
					}
					if (chunks.length > 0) return;
				}
			},
			cancel(reason) {
				return reader.cancel(reason);
			},
		});
	}

	async chatComplete(
		messages: IHistoryMessage[],
		appId?: string,
	): Promise<IResponse> {
		const baseUrl = getApiBaseUrl();
		const url = `${baseUrl}/api/v1/ai/copilot/chat`;

		const headers: HeadersInit = {
			"Content-Type": "application/json",
		};
		if (this.backend.auth?.user?.access_token) {
			headers["Authorization"] =
				`Bearer ${this.backend.auth.user.access_token}`;
		}

		const response = await fetch(url, {
			method: "POST",
			headers,
			body: JSON.stringify({ messages, app_id: appId }),
		});

		if (!response.ok) {
			throw new Error(`AI chat failed: ${response.status}`);
		}

		return response.json();
	}
}
