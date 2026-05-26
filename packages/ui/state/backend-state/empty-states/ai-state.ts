import type {
	IHistoryMessage,
	IResponse,
	IResponseChunk,
} from "@flow-like/flow-like-ui";
import type { IAIState } from "../ai-state";

export class EmptyAIState implements IAIState {
	streamChatComplete(
		messages: IHistoryMessage[],
	): Promise<ReadableStream<IResponseChunk[]>> {
		throw new Error("Method not implemented.");
	}
	chatComplete(messages: IHistoryMessage[]): Promise<IResponse> {
		throw new Error("Method not implemented.");
	}
}
