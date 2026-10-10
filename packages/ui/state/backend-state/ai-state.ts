import type { IHistoryMessage, IResponse, IResponseChunk } from "../../lib";

export interface IAIState {
	streamChatComplete(
		messages: IHistoryMessage[],
		appId?: string,
	): Promise<ReadableStream<IResponseChunk[]>>;
	/** Inline writing completion using the active profile's most cost-efficient available Bit. */
	chatComplete(messages: IHistoryMessage[], appId?: string): Promise<IResponse>;
}
