import type {
	IAIState,
	IHistoryMessage,
	IResponse,
	IResponseChunk,
} from "@flow-like/flow-like-ui";
import { type WebBackendRef, getApiBaseUrl } from "./api-utils";

export class WebAIState implements IAIState {
	constructor(private readonly backend: WebBackendRef) {}

	async streamChatComplete(
		messages: IHistoryMessage[],
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
			body: JSON.stringify({ messages }),
		});

		if (!response.ok) {
			throw new Error(`AI stream failed: ${response.status}`);
		}

		if (!response.body) {
			throw new Error("No response body for streaming");
		}

		const reader = response.body.getReader();
		const decoder = new TextDecoder();

		return new ReadableStream<IResponseChunk[]>({
			async pull(controller) {
				const { done, value } = await reader.read();
				if (done) {
					controller.close();
					return;
				}

				const text = decoder.decode(value, { stream: true });
				const lines = text.split("\n");
				const chunks: IResponseChunk[] = [];

				for (const line of lines) {
					if (line.startsWith("data: ")) {
						try {
							const data = JSON.parse(line.slice(6));
							chunks.push(data);
						} catch {
							// Ignore parse errors
						}
					}
				}

				if (chunks.length > 0) {
					controller.enqueue(chunks);
				}
			},
		});
	}

	async chatComplete(messages: IHistoryMessage[]): Promise<IResponse> {
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
			body: JSON.stringify({ messages }),
		});

		if (!response.ok) {
			throw new Error(`AI chat failed: ${response.status}`);
		}

		return response.json();
	}
}
