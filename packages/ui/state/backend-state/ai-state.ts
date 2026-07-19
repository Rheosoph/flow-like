import type { IHistoryMessage, IResponse, IResponseChunk } from "../../lib";

export interface IAIState {
	streamChatComplete(
		messages: IHistoryMessage[],
		appId?: string,
	): Promise<ReadableStream<IResponseChunk[]>>;
	chatComplete(messages: IHistoryMessage[], appId?: string): Promise<IResponse>;
}
