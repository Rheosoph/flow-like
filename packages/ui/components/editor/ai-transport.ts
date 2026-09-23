import {
	type ChatTransport,
	type FinishReason,
	type UIMessage,
	type UIMessageChunk,
	generateId,
} from "ai";
import { type IHistoryMessage, IRole } from "../../lib/schema/llm/history";
import type { IResponseChunk } from "../../lib/schema/llm/response-chunk";
import type { IAIState } from "../../state/backend-state/ai-state";

/** Keep both editor AI transports on the same app-attribution contract. */
export const streamEditorChat = (
	aiState: IAIState,
	messages: IHistoryMessage[],
	appId?: string,
) => aiState.streamChatComplete(messages, appId);

export const completeEditorChat = (
	aiState: IAIState,
	messages: IHistoryMessage[],
	appId?: string,
) => aiState.chatComplete(messages, appId);

const ROLES: Readonly<Record<UIMessage["role"], IRole>> = {
	assistant: IRole.Assistant,
	system: IRole.System,
	user: IRole.User,
};

const FINISH_REASONS: Readonly<Record<string, FinishReason>> = {
	content_filter: "content-filter",
	function_call: "tool-calls",
	length: "length",
	stop: "stop",
	tool_calls: "tool-calls",
};

/** The backend takes plain `{ role, content }` messages; only text parts carry content. */
export const toHistoryMessages = (
	messages: readonly UIMessage[],
): IHistoryMessage[] =>
	messages.map((message) => ({
		role: ROLES[message.role],
		content: message.parts
			.map((part) => (part.type === "text" ? part.text : ""))
			.join(""),
	}));

/**
 * Maps backend chunk batches onto one streamed text part. `text-start` waits for the
 * first non-empty delta so the chat stays `submitted` ("Thinking…") until text arrives.
 */
export const toUIMessageChunks = (
	source: ReadableStream<IResponseChunk[]>,
	textId: string,
): ReadableStream<UIMessageChunk> => {
	let started = false;
	let finishReason: FinishReason | undefined;

	return source.pipeThrough(
		new TransformStream<IResponseChunk[], UIMessageChunk>({
			transform(events, controller) {
				for (const event of events) {
					const choice = event?.choices?.[0];
					const delta = choice?.delta?.content;
					if (delta) {
						if (!started) {
							started = true;
							controller.enqueue({ type: "text-start", id: textId });
						}
						controller.enqueue({ type: "text-delta", id: textId, delta });
					}
					if (choice?.finish_reason && !finishReason) {
						finishReason = FINISH_REASONS[choice.finish_reason] ?? "other";
					}
				}
			},
			flush(controller) {
				if (started) controller.enqueue({ type: "text-end", id: textId });
				controller.enqueue({
					type: "finish",
					finishReason: finishReason ?? "stop",
				});
			},
		}),
	);
};

/** Settles as soon as `signal` aborts; a stream that arrives later is cancelled. */
const untilAborted = <T>(
	pending: Promise<ReadableStream<T>>,
	signal: AbortSignal | undefined,
): Promise<ReadableStream<T>> => {
	if (!signal) return pending;
	return new Promise((resolve, reject) => {
		const abort = () => {
			reject(new DOMException("Editor AI request was stopped", "AbortError"));
			pending.then((stream) => stream.cancel()).catch(() => undefined);
		};
		if (signal.aborted) {
			abort();
			return;
		}
		signal.addEventListener("abort", abort, { once: true });
		pending.then(
			(stream) => {
				signal.removeEventListener("abort", abort);
				resolve(stream);
			},
			(error: unknown) => {
				signal.removeEventListener("abort", abort);
				reject(error);
			},
		);
	});
};

export type EditorChatContext = Readonly<{
	aiState: IAIState;
	appId?: string;
}>;

/**
 * Chat transport for the editor AI menu over the host's `IAIState` stream. Stopping the
 * chat cancels the backend stream through the SDK's reader.
 */
export class BackendEditorChatTransport<M extends UIMessage>
	implements ChatTransport<M>
{
	constructor(private readonly context: EditorChatContext) {}

	sendMessages: ChatTransport<M>["sendMessages"] = async ({
		abortSignal,
		messages,
	}) => {
		const { aiState, appId } = this.context;
		const source = await untilAborted(
			streamEditorChat(aiState, toHistoryMessages(messages), appId),
			abortSignal,
		);
		return toUIMessageChunks(source, generateId());
	};

	reconnectToStream: ChatTransport<M>["reconnectToStream"] = async () => null;
}
