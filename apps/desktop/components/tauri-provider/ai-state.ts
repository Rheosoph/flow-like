import type {
	IHistoryMessage,
	IIntercomEvent,
	IResponse,
	IResponseChunk,
} from "@flow-like/flow-like-ui";
import type { IAIState } from "@flow-like/flow-like-ui/state/backend-state/ai-state";
import { Channel, invoke } from "@tauri-apps/api/core";
import type { TauriBackend } from "../tauri-provider";

export class AiState implements IAIState {
	constructor(private readonly backend: TauriBackend) {}

	async streamChatComplete(
		messages: IHistoryMessage[],
		appId?: string,
	): Promise<ReadableStream<IResponseChunk[]>> {
		const channel = new Channel<IIntercomEvent[]>();
		// Chunks keep arriving after a stop and after a cancel; the controller throws on them.
		let closed = false;

		// Create a ReadableStream that will be used to stream the response
		const stream = new ReadableStream<IResponseChunk[]>({
			start(controller) {
				channel.onmessage = (chunks: IIntercomEvent[]) => {
					if (closed) return;
					const responseChunks = chunks
						.filter((chunk) => chunk.event_type === "chunk")
						.map((chunk) => chunk.payload as IResponseChunk);
					controller.enqueue(responseChunks);
					if (
						responseChunks.some((chunk) =>
							Boolean(chunk?.choices?.[0]?.finish_reason),
						)
					) {
						closed = true;
						controller.close();
					}
				};
			},

			cancel() {
				// The background task finishes on its own; later chunks are dropped.
				closed = true;
			},
		});

		const token = this.backend.auth?.user?.access_token;

		const request = invoke<IResponse>("stream_chat_completion", {
			messages: messages,
			onChunk: channel,
			token: token,
			appId,
		});

		this.backend.backgroundTaskHandler(request);

		return stream;
	}

	async chatComplete(
		messages: IHistoryMessage[],
		appId?: string,
	): Promise<IResponse> {
		const token = this.backend.auth?.user?.access_token;
		const response = await invoke<IResponse>("chat_completion", {
			messages: messages,
			token: token,
			appId,
		});

		return response;
	}
}
