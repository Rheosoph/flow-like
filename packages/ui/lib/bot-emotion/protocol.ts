import type { BotExpression } from "../../components/bots/types";

export const BOT_EMOTION_MODEL = "Horizon-Labs/multilingual-emotions-small";
export const BOT_EMOTION_REVISION = "ac26dadbdafc26276e01093790104679bf33aec5";
export const MAX_DRAFT_CHARS = 768;
export const MAX_CONTEXT_CHARS = 384;
export const MAX_CONTEXT_MESSAGES = 4;

export interface EmotionMessage {
	role: "user" | "assistant";
	content: string;
}

export type BotEmotionMessage = EmotionMessage;

export interface EmotionInput {
	draft: string;
	messages: readonly EmotionMessage[];
}

export type EmotionScores = Record<string, number>;
export type ExpressionScores = Record<BotExpression, number>;
export type EmotionStatus = "idle" | "loading" | "ready" | "unavailable";

export interface EmotionLoadState {
	status: EmotionStatus;
	/** Download completion in [0, 1], when the server provides a total size. */
	progress?: number;
}

export interface EmotionWorkerRequest extends EmotionInput {
	id: number;
	warmup?: boolean;
}

export type EmotionWorkerResponse =
	| { type: "progress"; progress?: number }
	| { type: "ready" }
	| { type: "result"; id: number; scores: ExpressionScores }
	| { type: "error"; id: number };

export function boundEmotionInput(input: EmotionInput): EmotionInput {
	return {
		draft: input.draft.trim().slice(-MAX_DRAFT_CHARS),
		messages: input.messages
			.filter((message) => message.content.trim())
			.slice(-MAX_CONTEXT_MESSAGES)
			.map((message) => ({
				role: message.role,
				content: message.content.trim().slice(-MAX_CONTEXT_CHARS),
			})),
	};
}
